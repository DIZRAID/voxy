pub mod asr; // pub — используется примером examples/asr_smoke.rs
mod audio;
mod hotkey;
mod island;
mod output;
mod platform;
mod store;

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use asr::ModelStatus;
use store::SharedSettings;

pub enum WorkerMsg {
    LoadModel,
    /// Промежуточный кусок записи (стриминг): распознаётся, пока
    /// пользователь ещё говорит.
    Partial {
        session: u64,
        samples: Vec<f32>,
        sample_rate: u32,
    },
    /// Хвост записи: распознать, склеить с накопленными кусками и вставить.
    Final {
        session: u64,
        samples: Vec<f32>,
        sample_rate: u32,
        duration_ms: u64,
    },
    /// Запись отменена — накопленные куски сессии выбросить.
    CancelSession { session: u64 },
}

struct AppShared {
    settings: SharedSettings,
    model_status: ModelStatus,
    worker_tx: Mutex<Sender<WorkerMsg>>,
    ctrl_tx: Mutex<Sender<hotkey::Ctrl>>,
    capture: Arc<AtomicBool>,
}

// ---------------------------------------------------------------- commands

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
fn get_settings(state: State<AppShared>) -> store::Settings {
    store::read(&state.settings).clone()
}

#[tauri::command]
fn set_settings(
    app: AppHandle,
    state: State<AppShared>,
    mut new_settings: store::Settings,
) -> Result<(), String> {
    let (old_autostart, old_keep) = {
        let current = store::read(&state.settings);
        // Хоткей меняется только через захват клавиши (его сохраняет
        // контроллер). Страница может прислать устаревшее значение и
        // затереть только что выбранную клавишу — поэтому берём текущее.
        new_settings.hotkey = current.hotkey.clone();
        (current.autostart, current.history_keep.clone())
    };

    // Файл автозапуска трогаем только при реальном изменении настройки,
    // а не на каждое движение слайдера.
    if new_settings.autostart != old_autostart {
        let autolaunch = app.autolaunch();
        let result = if new_settings.autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(e) = result {
            log::warn!("автозапуск: {e}");
        }
    }

    *store::write(&state.settings) = new_settings.clone();
    store::save_settings(&app, &new_settings);
    if new_settings.history_keep != old_keep {
        store::prune_history(&app, &new_settings.history_keep);
    }
    Ok(())
}

/// Отмена текущей записи (кнопка Cancel на островке).
#[tauri::command]
fn cancel_recording(state: State<AppShared>) {
    let _ = lock(&state.ctrl_tx).send(hotkey::Ctrl::Cancel);
}

// Команды с диском/CoreAudio — (async): иначе Tauri выполняет их на
// главном потоке и задерживает события островка.
#[tauri::command(async)]
fn list_mics() -> Vec<String> {
    audio::list_input_devices()
}

#[tauri::command(async)]
fn get_history(app: AppHandle) -> Vec<store::HistoryEntry> {
    store::load_history(&app)
}

#[tauri::command(async)]
fn clear_history(app: AppHandle) {
    store::clear_history(&app);
}

#[tauri::command(async)]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_model_status(state: State<AppShared>) -> serde_json::Value {
    json!({ "status": asr::status_name(state.model_status.load(Ordering::Relaxed)) })
}

#[tauri::command]
fn start_model_download(app: AppHandle, state: State<AppShared>) {
    let worker_tx = lock(&state.worker_tx).clone();
    asr::spawn_download(app, state.model_status.clone(), worker_tx);
}

#[tauri::command]
fn begin_hotkey_capture(state: State<AppShared>) {
    state.capture.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn cancel_hotkey_capture(state: State<AppShared>) {
    state.capture.store(false, Ordering::Relaxed);
}

/// Геометрия выреза для island.js — размеры пилюли строятся от неё.
#[tauri::command]
fn island_metrics(metrics: State<island::IslandMetrics>) -> island::IslandMetrics {
    *metrics
}

#[tauri::command]
fn permissions_status() -> serde_json::Value {
    json!({
        "accessibility": platform::accessibility_trusted(),
        "input_monitoring": platform::input_monitoring_granted(),
    })
}

#[tauri::command]
fn open_permission_settings(which: String) {
    match which.as_str() {
        "input-monitoring" => platform::open_input_monitoring_settings(),
        _ => platform::open_accessibility_settings(),
    }
}

/// Открывает ссылку в браузере. Только https и только доверенные домены
/// (страницы модели/проекта) — не общий проходной для произвольных URL.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    const ALLOWED: [&str; 3] = ["huggingface.co", "github.com", "www.nvidia.com"];
    let ok = url::host(&url)
        .map(|h| ALLOWED.contains(&h.as_str()))
        .unwrap_or(false);
    if !url.starts_with("https://") || !ok {
        return Err("url not allowed".into());
    }
    platform::open_external(&url);
    Ok(())
}

/// Крошечный парсер хоста, чтобы не тянуть крейт url ради одной проверки.
mod url {
    pub fn host(url: &str) -> Option<String> {
        let rest = url.strip_prefix("https://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let host = &rest[..end];
        if host.is_empty() || host.contains('@') || host.contains(':') {
            return None;
        }
        Some(host.to_ascii_lowercase())
    }
}

#[tauri::command]
fn model_info(app: AppHandle, state: State<AppShared>) -> serde_json::Value {
    let paths = asr::model_paths(&app);
    let size: u64 = std::fs::read_dir(&paths.dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0);
    json!({
        "status": asr::status_name(state.model_status.load(Ordering::Relaxed)),
        "dir": paths.dir.to_string_lossy(),
        "size_bytes": size,
    })
}

// ------------------------------------------------------------------ worker

fn spawn_worker(
    app: AppHandle,
    rx: Receiver<WorkerMsg>,
    model_status: ModelStatus,
    settings: SharedSettings,
) {
    std::thread::spawn(move || {
        let mut transcriber: Option<asr::Transcriber> = None;
        // Накопленные куски текущей сессии записи (стриминг).
        let mut current_session: u64 = 0;
        let mut parts: Vec<String> = Vec::new();

        for msg in rx {
            // Паника на одном сообщении не должна убивать worker: иначе
            // островок навсегда зависает в «Transcribing…».
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match msg {
                WorkerMsg::LoadModel => {
                    asr::emit_status(&app, &model_status, asr::STATUS_LOADING, json!({}));
                    let paths = asr::model_paths(&app);
                    match asr::Transcriber::load(&paths) {
                        Ok(t) => {
                            let t0 = Instant::now();
                            t.warmup();
                            log::info!("модель загружена, прогрев {:?}", t0.elapsed());
                            transcriber = Some(t);
                            asr::emit_status(&app, &model_status, asr::STATUS_READY, json!({}));
                        }
                        Err(e) => {
                            log::error!("загрузка модели: {e:#}");
                            asr::emit_status(
                                &app,
                                &model_status,
                                asr::STATUS_ERROR,
                                json!({ "message": format!("{e:#}") }),
                            );
                        }
                    }
                }
                WorkerMsg::Partial {
                    session,
                    samples,
                    sample_rate,
                } => {
                    let Some(t) = transcriber.as_ref() else { return };
                    if session != current_session {
                        current_session = session;
                        parts.clear();
                    }
                    let t0 = Instant::now();
                    let text = t.transcribe_long(&samples, sample_rate);
                    log::info!(
                        "промежуточный кусок распознан за {:?}: {} символов",
                        t0.elapsed(),
                        text.chars().count()
                    );
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
                WorkerMsg::Final {
                    session,
                    samples,
                    sample_rate,
                    duration_ms,
                } => {
                    let (sounds, history_keep) = {
                        let s = store::read(&settings);
                        (s.sounds, s.history_keep.clone())
                    };
                    let Some(t) = transcriber.as_ref() else {
                        island::set_state(&app, "error", Some("Model is not loaded".into()));
                        return;
                    };
                    if session != current_session {
                        current_session = session;
                        parts.clear();
                    }

                    let t0 = Instant::now();
                    let tail = t.transcribe_long(&samples, sample_rate);
                    if !tail.is_empty() {
                        parts.push(tail);
                    }
                    let text = parts.join(" ").trim().to_string();
                    parts.clear();
                    log::info!(
                        "финал: хвост за {:?}, всего {} символов (запись {} мс)",
                        t0.elapsed(),
                        text.chars().count(),
                        duration_ms
                    );

                    if text.is_empty() {
                        island::set_state(&app, "error", Some("Didn't catch that".into()));
                        if sounds {
                            platform::play(platform::Sound::Error);
                        }
                        return;
                    }

                    match output::insert_text(&text) {
                        // Текст уже на месте — островок схлопывается сразу,
                        // без промежуточной «галочки».
                        Ok(()) => island::set_state(&app, "idle", None),
                        Err(e) => {
                            log::error!("вставка: {e:#}");
                            island::set_state(
                                &app,
                                "error",
                                Some("Paste failed — text kept in clipboard".into()),
                            );
                        }
                    }

                    store::push_history(
                        &app,
                        store::HistoryEntry {
                            text,
                            ts_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0),
                            duration_ms,
                        },
                        &history_keep,
                    );
                }
                WorkerMsg::CancelSession { session } => {
                    if session == current_session {
                        parts.clear();
                    }
                }
            }
            }));

            if outcome.is_err() {
                log::error!("паника в worker распознавания — сессия сброшена");
                parts.clear();
                if transcriber.is_none() {
                    asr::emit_status(
                        &app,
                        &model_status,
                        asr::STATUS_ERROR,
                        json!({ "message": "model failed to load" }),
                    );
                }
                island::set_state(&app, "error", Some("Something went wrong".into()));
            }
        }
    });
}

// ------------------------------------------------------------------- setup

/// Логи в файл (macOS: ~/Library/Logs/Typely.log, Windows:
/// %LOCALAPPDATA%\Typely.log) — диагностика без запуска через пайпы.
/// VOICE_LOG_STDERR=1 — в stderr.
fn init_logging() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if std::env::var("VOICE_LOG_STDERR").is_err() {
        let path = if cfg!(target_os = "windows") {
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|d| std::path::Path::new(&d).join("Typely.log"))
        } else {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join("Library/Logs/Typely.log"))
        };
        if let Some(path) = path {
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                builder.target(env_logger::Target::Pipe(Box::new(file)));
            }
        }
    }
    builder.init();
}

/// Окно настроек создаётся только при открытии и уничтожается при
/// закрытии: скрытое окно держало отдельный WebKit-процесс всё время
/// работы приложения (25 МБ в покое, до ~200 МБ пика).
fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "settings")
        .cloned()
    else {
        log::error!("в tauri.conf.json нет окна settings");
        return;
    };
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(e) => log::error!("не удалось создать окно настроек: {e}"),
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let settings_item = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit Typely").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&settings_item)
        .separator()
        .item(&quit_item)
        .build()?;

    // Template-глиф: macOS сам перекрашивает его под тему меню-бара
    // (чёрный на светлой, белый на тёмной) — как у системных иконок.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Typely — local dictation")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    init_logging();

    let mut builder = tauri::Builder::default()
        // Второй запуск не создаёт второй процесс (и второй event tap!),
        // а просто показывает окно настроек уже работающего экземпляра.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            list_mics,
            get_history,
            clear_history,
            copy_text,
            get_model_status,
            start_model_download,
            begin_hotkey_capture,
            cancel_hotkey_capture,
            cancel_recording,
            permissions_status,
            open_permission_settings,
            island_metrics,
            open_url,
            model_info,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            platform::init(&handle);
            let settings: SharedSettings =
                Arc::new(RwLock::new(store::load_settings(&handle)));
            let model_status: ModelStatus = Arc::new(AtomicU8::new(asr::STATUS_MISSING));
            let (worker_tx, worker_rx) = channel::<WorkerMsg>();

            spawn_worker(
                handle.clone(),
                worker_rx,
                model_status.clone(),
                settings.clone(),
            );
            let hk = hotkey::spawn(
                handle.clone(),
                settings.clone(),
                model_status.clone(),
                worker_tx.clone(),
            );

            // Замер физического выреза — до создания панели, на главном потоке.
            app.manage(island::measure_metrics());
            island::create(&handle)?;
            build_tray(app)?;
            let keep = store::read(&settings).history_keep.clone();
            store::prune_history(&handle, &keep);

            if asr::model_paths(&handle).exists() {
                let _ = worker_tx.send(WorkerMsg::LoadModel);
            } else {
                asr::spawn_download(handle.clone(), model_status.clone(), worker_tx.clone());
            }

            app.manage(AppShared {
                settings,
                model_status,
                worker_tx: Mutex::new(worker_tx),
                ctrl_tx: Mutex::new(hk.ctrl_tx),
                capture: hk.capture,
            });

            // Без Accessibility/Input Monitoring ни хоткей, ни вставка
            // не работают — сразу показываем настройки с баннером.
            if !platform::accessibility_trusted() || !platform::input_monitoring_granted() {
                show_settings(&handle);
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("ошибка запуска tauri")
        .run(|_app, event| {
            // Приложение живёт в меню-баре: закрытие окна настроек не должно
            // его завершать. Выход — только явный (пункт Quit, code = Some).
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
