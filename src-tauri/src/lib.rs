// pub — используются тестовой утилитой examples/model_smoke.rs
pub mod asr;
pub mod models;
mod audio;
mod discovery;
mod hotkey;
mod island;
mod online;
mod output;
mod platform;
mod store;
mod worker;

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use asr::ModelStatus;
use store::SharedSettings;
pub use worker::WorkerMsg;

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
        // контроллер), активная модель — только через model_activate.
        // Страница может прислать устаревшие значения и затереть только
        // что сделанный выбор — поэтому берём текущие.
        new_settings.hotkey = current.hotkey.clone();
        new_settings.active_model = current.active_model.clone();
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
    let active = store::read(&state.settings).active_model.clone();
    let (name, vendor) = engine_label(&active);
    json!({
        "status": asr::status_name(state.model_status.load(Ordering::Relaxed)),
        "model": active,
        "name": name,
        "vendor": vendor,
    })
}

/// Имя и производитель активного движка для карточки статуса в окне
/// настроек. Без обращения к Связке ключей (в отличие от models_overview),
/// поэтому его можно звать при каждом открытии окна.
fn engine_label(active: &str) -> (String, String) {
    match active.strip_prefix(store::ONLINE_PREFIX) {
        Some(p) => match online::provider(p) {
            Some(p) => (format!("{} {}", p.name, p.model), "Online".to_string()),
            None => (active.to_string(), String::new()),
        },
        None => match models::find(active) {
            Some(m) => (m.name.clone(), m.vendor.clone()),
            None => (active.to_string(), String::new()),
        },
    }
}

// ------------------------------------------------------- менеджер моделей

/// Всё для экрана моделей: каталог с состоянием установки и провайдеры.
#[tauri::command(async)]
fn models_overview(app: AppHandle) -> serde_json::Value {
    let state = app.state::<AppShared>();
    let (active, unload_after_min) = {
        let s = store::read(&state.settings);
        (s.active_model.clone(), s.unload_after_min)
    };
    let models: Vec<_> = models::catalog()
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "name": m.name, "vendor": m.vendor, "labels": m.labels,
                "languages": m.languages, "languages_note": m.languages_note,
                "english_only": m.english_only, "ram_mb": m.ram_mb, "size_mb": m.size_mb,
                "speed": m.speed,
                "download_mb": (m.download_bytes() as f64 / 1e6).round(),
                "license": m.license, "homepage": m.homepage,
                "installed": models::is_installed(&app, m),
                "downloading": models::downloads().is_active(&m.id),
            })
        })
        .collect();
    let providers: Vec<_> = online::PROVIDERS
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).unwrap_or_default();
            v["has_key"] = json!(online::has_key(p.id));
            v
        })
        .collect();
    json!({
        "active": active,
        "status": asr::status_name(state.model_status.load(Ordering::Relaxed)),
        "unload_after_min": unload_after_min,
        "models": models,
        "providers": providers,
        "discovered": discovery::cached(&app),
        "discovery_url": discovery::RELEASE_PAGE,
    })
}

fn models_changed(app: &AppHandle) {
    let _ = app.emit_to("settings", "models-changed", ());
}

/// Скачивание в фоне; если скачанная модель активна — сразу загружается.
fn download_model(app: &AppHandle, spec: &'static models::ModelSpec) {
    let state = app.state::<AppShared>();
    let worker_tx = lock(&state.worker_tx).clone();
    let settings = state.settings.clone();
    let app2 = app.clone();
    models::start_download(app.clone(), spec, move |ok| {
        models_changed(&app2);
        if ok && store::read(&settings).active_model == spec.id {
            let _ = worker_tx.send(WorkerMsg::Reload);
        }
    });
    models_changed(app);
}

#[tauri::command]
fn model_download(app: AppHandle, id: String) -> Result<(), String> {
    let spec = models::find(&id).ok_or("Unknown model")?;
    download_model(&app, spec);
    Ok(())
}

#[tauri::command]
fn model_cancel_download(id: String) {
    models::downloads().cancel(&id);
}

#[tauri::command(async)]
fn model_delete(app: AppHandle, id: String) -> Result<(), String> {
    let spec = models::find(&id).ok_or("Unknown model")?;
    if store::read(&app.state::<AppShared>().settings).active_model == id {
        return Err("Switch to another model before deleting this one".into());
    }
    models::delete(&app, spec).map_err(|e| format!("{e:#}"))?;
    log::info!("модель {id} удалена");
    models_changed(&app);
    Ok(())
}

/// Сделать активным движком локальную модель (id) или онлайн-провайдера
/// ("online:<id>").
#[tauri::command]
fn model_activate(app: AppHandle, id: String) -> Result<(), String> {
    if let Some(p) = id.strip_prefix(store::ONLINE_PREFIX) {
        let provider = online::provider(p).ok_or("Unknown provider")?;
        if !online::has_key(provider.id) {
            return Err(format!("Add an API key for {} first", provider.name));
        }
    } else {
        let spec = models::find(&id).ok_or("Unknown model")?;
        if !models::is_installed(&app, spec) {
            return Err("Download the model first".into());
        }
    }
    let state = app.state::<AppShared>();
    {
        let mut s = store::write(&state.settings);
        s.active_model = id.clone();
        store::save_settings(&app, &s);
    }
    log::info!("активный движок: {id}");
    let _ = lock(&state.worker_tx).send(WorkerMsg::Reload);
    models_changed(&app);
    Ok(())
}

/// Проверяет ключ у провайдера и только потом сохраняет в Связку ключей.
#[tauri::command(async)]
fn provider_save_key(app: AppHandle, provider: String, key: String) -> Result<(), String> {
    let p = online::provider(&provider).ok_or("Unknown provider")?;
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste an API key".into());
    }
    online::verify_key(p, key).map_err(|e| e.to_string())?;
    online::set_key(p.id, key).map_err(|e| e.to_string())?;
    models_changed(&app);
    Ok(())
}

#[tauri::command(async)]
fn provider_delete_key(app: AppHandle, provider: String) -> Result<(), String> {
    let p = online::provider(&provider).ok_or("Unknown provider")?;
    online::delete_key(p.id).map_err(|e| e.to_string())?;
    // Ключ удалён у активного провайдера — возвращаемся к локальной модели.
    let state = app.state::<AppShared>();
    if store::read(&state.settings).online_provider() == Some(p.id) {
        let fallback = first_installed_model(&app).unwrap_or(models::DEFAULT_MODEL);
        {
            let mut s = store::write(&state.settings);
            s.active_model = fallback.to_string();
            store::save_settings(&app, &s);
        }
        let _ = lock(&state.worker_tx).send(WorkerMsg::Reload);
    }
    models_changed(&app);
    Ok(())
}

fn first_installed_model(app: &AppHandle) -> Option<&'static str> {
    models::catalog()
        .models
        .iter()
        .find(|m| models::is_installed(app, m))
        .map(|m| m.id.as_str())
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
    const ALLOWED: [&str; 6] = [
        "huggingface.co",
        "github.com",
        "www.nvidia.com",
        "platform.openai.com",
        "console.groq.com",
        "elevenlabs.io",
    ];
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

// ------------------------------------------------------------------- setup

/// Логи в файл (macOS: ~/Library/Logs/Voxy.log, Windows:
/// %LOCALAPPDATA%\Voxy.log) — диагностика без запуска через пайпы.
/// VOXY_LOG_STDERR=1 — в stderr.
fn init_logging() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if std::env::var("VOXY_LOG_STDERR").is_err() {
        let path = if cfg!(target_os = "windows") {
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|d| std::path::Path::new(&d).join("Voxy.log"))
        } else {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join("Library/Logs/Voxy.log"))
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
    // Стекло (прозрачное окно + NSVisualEffectView из windowEffects) — только
    // macOS. Порт на Windows не проверен: там окно непрозрачное, фон рисует
    // CSS html[data-platform="win"], а цвет ниже убирает белую вспышку.
    let config = if cfg!(target_os = "macos") {
        config
    } else {
        let mut config = config;
        config.transparent = false;
        config.window_effects = None;
        config.background_color = Some(tauri::window::Color(26, 22, 43, 255));
        config
    };
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            #[cfg(target_os = "macos")]
            force_dark_appearance(&window);
            // Окно закрыли посреди захвата хоткея — снимаем флаг, иначе
            // следующая клавиша в любом приложении стала бы хоткеем.
            if let Some(state) = app.try_state::<AppShared>() {
                let capture = state.capture.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        capture.store(false, Ordering::Relaxed);
                    }
                });
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(e) => log::error!("не удалось создать окно настроек: {e}"),
    }
}

/// Тёмное стекло только для окна настроек. `"theme": "Dark"` в конфиге не
/// подходит: tao ставит NSApp.appearance на всё приложение (меню трея,
/// островок, системные алерты). NSWindow.appearance действует на одно окно.
#[cfg(target_os = "macos")]
fn force_dark_appearance(window: &tauri::WebviewWindow) {
    let result = window.with_webview(|webview| {
        use objc2_app_kit::{
            NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSWindow,
        };
        // SAFETY: with_webview выполняет замыкание на главном потоке, а
        // ns_window() — живой NSWindow этого окна.
        unsafe {
            let ns_window: &NSWindow = &*webview.ns_window().cast::<NSWindow>();
            let dark = NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua);
            ns_window.setAppearance(dark.as_deref());
        }
    });
    if let Err(e) = result {
        log::warn!("тёмное оформление окна настроек: {e}");
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let settings_item = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit Voxy").build(app)?;
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
        .tooltip("Voxy — local dictation")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

enum StartupEngine {
    Ready,
    NeedsDownload(&'static models::ModelSpec),
}

/// Проверяет активный движок при запуске и чинит настройки, если он
/// недоступен: модель удалили вручную, она пропала из каталога, у
/// онлайн-провайдера нет ключа. Тогда — любая установленная модель,
/// а на чистой машине — скачивание модели по умолчанию.
fn resolve_startup_engine(app: &AppHandle, settings: &SharedSettings) -> StartupEngine {
    let active = store::read(settings).active_model.clone();
    let usable = match active.strip_prefix(store::ONLINE_PREFIX) {
        Some(p) => online::provider(p).is_some() && online::has_key(p),
        None => models::find(&active).is_some_and(|m| models::is_installed(app, m)),
    };
    if usable {
        return StartupEngine::Ready;
    }

    let fallback = first_installed_model(app);
    let id = fallback.unwrap_or(models::DEFAULT_MODEL);
    if id != active {
        log::warn!("активный движок {active} недоступен, переключаюсь на {id}");
        let mut s = store::write(settings);
        s.active_model = id.to_string();
        store::save_settings(app, &s);
    }
    match fallback {
        Some(_) => StartupEngine::Ready,
        None => StartupEngine::NeedsDownload(
            models::find(models::DEFAULT_MODEL).expect("модель по умолчанию есть в каталоге"),
        ),
    }
}

pub fn run() {
    init_logging();

    let builder = tauri::Builder::default()
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
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            list_mics,
            get_history,
            clear_history,
            copy_text,
            get_model_status,
            models_overview,
            model_download,
            model_cancel_download,
            model_delete,
            model_activate,
            provider_save_key,
            provider_delete_key,
            begin_hotkey_capture,
            cancel_hotkey_capture,
            cancel_recording,
            permissions_status,
            open_permission_settings,
            island_metrics,
            open_url,
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

            worker::spawn(
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

            let active = resolve_startup_engine(&handle, &settings);

            app.manage(AppShared {
                settings,
                model_status,
                worker_tx: Mutex::new(worker_tx.clone()),
                ctrl_tx: Mutex::new(hk.ctrl_tx),
                capture: hk.capture,
            });

            discovery::spawn_check(handle.clone());

            match active {
                // Первый запуск: ни одной модели на диске — качаем модель
                // по умолчанию (после скачивания worker загрузит её сам).
                StartupEngine::NeedsDownload(spec) => download_model(&handle, spec),
                StartupEngine::Ready => {
                    let _ = worker_tx.send(WorkerMsg::EnsureLoaded);
                }
            }

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

#[cfg(test)]
mod tests {
    use super::engine_label;

    #[test]
    fn engine_label_names() {
        let label = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            engine_label("parakeet-tdt-0.6b-v3"),
            label("Parakeet TDT 0.6B v3", "NVIDIA")
        );
        assert_eq!(
            engine_label("online:groq"),
            label("Groq whisper-large-v3-turbo", "Online")
        );
        // неизвестный id показывается как есть
        assert_eq!(engine_label("online:nope"), label("online:nope", ""));
        assert_eq!(engine_label("no-such-model"), label("no-such-model", ""));
    }

    /// Окно настроек — нативное стекло: прозрачное окно, NSVisualEffectView
    /// со скруглением 22 (= border-radius корня в settings.css) и
    /// «светофоры» над боковой панелью. show_settings берёт всё из конфига.
    #[test]
    fn settings_window_is_glass() {
        use tauri::utils::{config::Color, WindowEffect, WindowEffectState};
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let w = config
            .app
            .windows
            .iter()
            .find(|w| w.label == "settings")
            .expect("settings");
        assert!(!w.create && w.transparent && w.decorations);
        assert_eq!(w.background_color, Some(Color(0, 0, 0, 0)));
        let tl = w
            .traffic_light_position
            .as_ref()
            .expect("trafficLightPosition");
        assert_eq!((tl.x, tl.y), (19.0, 28.0));
        let fx = w.window_effects.as_ref().expect("windowEffects");
        assert_eq!(fx.effects, vec![WindowEffect::UnderWindowBackground]);
        assert_eq!(fx.state, Some(WindowEffectState::Active));
        assert_eq!(fx.radius, Some(22.0));
        assert!(
            w.theme.is_none(),
            "theme на macOS меняет оформление всего приложения"
        );
        assert!(
            config.app.macos_private_api,
            "прозрачное окно требует macOSPrivateApi"
        );
    }

    /// Релизы подписываются (release.yml, APPLE_SIGNING_IDENTITY) с hardened
    /// runtime, а под ним микрофон без entitlement audio-input молча пишет
    /// тишину. Локальные неподписанные сборки entitlements не используют.
    #[test]
    fn signed_builds_keep_microphone() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(
            config.bundle.macos.entitlements.as_deref(),
            Some("Entitlements.plist")
        );
        let plist = include_str!("../Entitlements.plist");
        let key = "<key>com.apple.security.device.audio-input</key>";
        let after = &plist[plist.find(key).expect("audio-input") + key.len()..];
        assert!(after.trim_start().starts_with("<true/>"));
    }
}
