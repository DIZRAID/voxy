// pub — используются тестовой утилитой examples/model_smoke.rs
pub mod asr;
pub mod models;
#[cfg(test)]
mod app_commands;
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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::{WindowConfig, WindowEffectsConfig};
use tauri::utils::WindowEffect;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl};
use tauri_plugin_autostart::ManagerExt;

use asr::ModelStatus;
use store::SharedSettings;
pub use worker::WorkerMsg;

struct AppShared {
    settings: SharedSettings,
    model_status: ModelStatus,
    worker_tx: Mutex<Sender<WorkerMsg>>,
    ctrl_tx: Mutex<Sender<hotkey::Ctrl>>,
    capture: Arc<hotkey::Capture>,
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

/// История с учётом срока хранения: записи старше срока не показываются
/// и тут же удаляются с диска, даже если приложение работает неделями.
#[tauri::command(async)]
fn get_history(app: AppHandle, state: State<AppShared>) -> Vec<store::HistoryEntry> {
    let keep = store::read(&state.settings).history_keep.clone();
    store::load_history(&app, &keep)
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
    let key = online::clean_key(&key)?;
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

/// Следующая клавиша (в течение hotkey::CAPTURE_WINDOW и пока окно
/// настроек в фокусе) станет хоткеем; итог приходит событием
/// hotkey-captured.
#[tauri::command]
fn begin_hotkey_capture(state: State<AppShared>) {
    state.capture.begin();
}

#[tauri::command]
fn cancel_hotkey_capture(state: State<AppShared>) {
    state.capture.cancel();
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
    if !url::allowed(&url) {
        return Err("url not allowed".into());
    }
    platform::open_external(&url);
    Ok(())
}

/// Крошечный парсер хоста, чтобы не тянуть крейт url ради одной проверки.
mod url {
    const ALLOWED_HOSTS: [&str; 6] = [
        "huggingface.co",
        "github.com",
        "www.nvidia.com",
        "platform.openai.com",
        "console.groq.com",
        "elevenlabs.io",
    ];

    /// https, хост из списка и только «безопасные» символы: URL уходит
    /// аргументом в open / explorer.exe, а explorer по-своему разбирает
    /// запятые и кавычки. Пробелы, управляющие символы и не-ASCII не нужны
    /// ни одной ссылке приложения.
    pub fn allowed(url: &str) -> bool {
        url.len() <= 2048
            && url
                .bytes()
                .all(|b| b.is_ascii_graphic() && !b"\"\\<>^`{|},".contains(&b))
            && host(url).is_some_and(|h| ALLOWED_HOSTS.contains(&h.as_str()))
    }

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

/// Идентификатор приложения ("identifier" в tauri.conf.json) — имя папки
/// данных. Продублирован намеренно: логирование поднимается до Tauri, когда
/// конфиг ещё не прочитан. Тест log_folder_matches_identifier следит, чтобы
/// строки не разошлись.
const APP_IDENTIFIER: &str = "com.dizraid.voice";

/// Файл лога. macOS: ~/Library/Logs/Voxy.log (там его ищет Console.app).
/// Windows: %LOCALAPPDATA%\com.dizraid.voice\Voxy.log — рядом с настройками,
/// историей и моделями (store::data_dir), так что всё приложение лежит в
/// одной папке, которую деинсталлятор и install.ps1 -Uninstall стирают целиком.
fn log_file_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let dir = Path::new(&std::env::var_os("LOCALAPPDATA")?).join(APP_IDENTIFIER);
        // На первом запуске папки ещё нет (store::data_dir создаст её позже).
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("Voxy.log"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|h| Path::new(&h).join("Library/Logs/Voxy.log"))
    }
}

/// Логи в файл (log_file_path) — диагностика без запуска через пайпы.
/// VOXY_LOG_STDERR=1 — в stderr.
fn init_logging() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    // Отладочный лог ureq печатает заголовки запросов и скрывает только
    // Authorization и Cookie: ключ ElevenLabs (xi-api-key) попал бы в файл.
    // Поэтому ureq — не подробнее warn, что бы ни стояло в RUST_LOG
    // (директива для того же модуля заменяет пришедшую из RUST_LOG).
    builder.filter_module("ureq", log::LevelFilter::Warn);
    builder.filter_module("ureq::unit", log::LevelFilter::Warn);
    if std::env::var("VOXY_LOG_STDERR").is_err() {
        if let Some(path) = log_file_path() {
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

/// Mica — только Windows 11 (сборка 22000+). window-vibrancy молча
/// проглатывает ошибку, и на Windows 10 осталось бы прозрачное окно, сквозь
/// которое видно рабочий стол, — поэтому проверяем версию сами.
const MICA_MIN_BUILD: u32 = 22000;

fn mica_supported(windows_build: u32) -> bool {
    windows_build >= MICA_MIN_BUILD
}

/// Номер сборки Windows (RtlGetVersion: без манифеста совместимости, в
/// отличие от GetVersionEx). Вне Windows не вызывается.
#[cfg(target_os = "windows")]
fn windows_build() -> u32 {
    windows_version::OsVersion::current().build
}

#[cfg(not(target_os = "windows"))]
fn windows_build() -> u32 {
    0
}

/// Окно настроек на Windows (конфиг в tauri.conf.json описывает macOS-стекло).
///
/// Заголовок — нативный, но всегда тёмный: на Windows `theme` действует на
/// одно окно (tao: DWMWA_USE_IMMERSIVE_DARK_MODE, плюс цветовая схема
/// WebView2), а не на всё приложение, как на macOS, где его поэтому и нет в
/// конфиге. Без него tao следует системной теме приложений, по умолчанию
/// светлой: белый заголовок над тёмным окном.
///
/// - `mica` (Windows 11): прозрачное окно, фон — тёмная Mica от DWM (Tauri
///   применяет windowEffects сразу после создания окна).
/// - иначе (Windows 10): непрозрачное окно цвета фона страницы; цвет
///   заодно убирает белую вспышку до первой отрисовки.
///
/// Страница узнаёт вариант из ?backdrop= (boot.js → html[data-backdrop]).
fn windows_settings_config(mut config: WindowConfig, mica: bool) -> WindowConfig {
    config.theme = Some(tauri::Theme::Dark);
    if mica {
        config.transparent = true;
        config.window_effects = Some(WindowEffectsConfig {
            effects: vec![WindowEffect::MicaDark],
            state: None,
            radius: None,
            color: None,
            // Только Liquid Glass на macOS 27+; к Mica отношения не имеет.
            interactive: false,
        });
        config.background_color = None;
        config.url = WebviewUrl::App("settings.html?backdrop=mica".into());
    } else {
        config.transparent = false;
        config.window_effects = None;
        config.background_color = Some(tauri::window::Color(26, 22, 43, 255));
        config.url = WebviewUrl::App("settings.html?backdrop=opaque".into());
    }
    config
}

/// Когда создавалось окно настроек (только Windows, см. reveal_existing_settings).
static SETTINGS_CREATED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Сколько ждать конца загрузки страницы, прежде чем показать окно как есть.
const SETTINGS_LOAD_GRACE: Duration = Duration::from_secs(3);

/// Окно настроек уже есть — показать его сейчас? Видимое — да (вывести
/// вперёд). Скрытым оно бывает только на Windows, между созданием и концом
/// загрузки страницы: его покажет on_page_load, а ранний show() вернул бы
/// вспышку пустого окна. Так бывает при двойном клике по значку в трее
/// (Click, DoubleClick и ещё Click подряд). Если загрузка не закончилась за
/// SETTINGS_LOAD_GRACE (упал процесс WebView2?), показываем как есть, чтобы
/// окно не осталось невидимым навсегда.
fn reveal_existing_settings(visible: bool, since_created: Duration) -> bool {
    visible || since_created >= SETTINGS_LOAD_GRACE
}

/// Окно настроек создаётся только при открытии и уничтожается при
/// закрытии: скрытое окно держало отдельный WebKit-процесс всё время
/// работы приложения (25 МБ в покое, до ~200 МБ пика).
fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        if cfg!(target_os = "windows") {
            let visible = window.is_visible().unwrap_or(true);
            let since_created = lock(&SETTINGS_CREATED_AT).map_or(Duration::MAX, |t| t.elapsed());
            if !reveal_existing_settings(visible, since_created) {
                return;
            }
            // tao не активирует свёрнутое окно (set_focus пропускает
            // минимизированные), и клик по трею ничего бы не делал.
            let _ = window.unminimize();
        }
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
    // macOS: стекло из конфига как есть (прозрачное окно + NSVisualEffectView
    // из windowEffects). Windows: см. windows_settings_config.
    let config = if cfg!(target_os = "windows") {
        let build = windows_build();
        let mica = mica_supported(build);
        log::info!(
            "окно настроек: Windows build {build}, фон {}",
            if mica { "Mica" } else { "непрозрачный" }
        );
        windows_settings_config(config, mica)
    } else {
        config
    };
    let mut builder = match tauri::WebviewWindowBuilder::from_config(app, &config) {
        Ok(builder) => builder,
        Err(e) => {
            log::error!("не удалось создать окно настроек: {e}");
            return;
        }
    };
    // Windows: окно (в конфиге visible: false) показывается, когда страница
    // загрузилась, — иначе мелькнул бы пустой фон (тёмный у непрозрачного
    // окна, сквозной у Mica). macOS показывает сразу, как и раньше.
    if cfg!(target_os = "windows") {
        *lock(&SETTINGS_CREATED_AT) = Some(Instant::now());
        builder = builder.on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.show();
                let _ = window.set_focus();
            }
        });
    }
    match builder.build() {
        Ok(window) => {
            #[cfg(target_os = "macos")]
            force_dark_appearance(&window);
            // Захват хоткея принимает клавишу, только пока это окно в
            // фокусе. Окно закрыли посреди захвата — снимаем его, иначе
            // следующая клавиша в любом приложении стала бы хоткеем.
            if let Some(state) = app.try_state::<AppShared>() {
                let capture = state.capture.clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Focused(focused) => {
                        capture.set_settings_focused(*focused);
                    }
                    tauri::WindowEvent::Destroyed => {
                        capture.set_settings_focused(false);
                        capture.cancel();
                    }
                    _ => {}
                });
            }
            if !cfg!(target_os = "windows") {
                let _ = window.show();
                let _ = window.set_focus();
            }
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

    let icon = tauri::image::Image::from_bytes(TRAY_ICON)?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        // Template перекрашивает только macOS; на Windows флаг ничего не делает.
        .icon_as_template(cfg!(target_os = "macos"))
        // Windows: меню — по правой кнопке, левая открывает настройки (как
        // у системных значков). macOS: меню по левой, как в строке меню.
        .show_menu_on_left_click(!cfg!(target_os = "windows"))
        .tooltip("Voxy — local dictation")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if cfg!(target_os = "windows") && tray_click_opens_settings(&event) {
                show_settings(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// macOS: template-глиф, macOS сам перекрашивает его под тему меню-бара
/// (чёрный на светлой, белый на тёмной) — как у системных иконок.
#[cfg(target_os = "macos")]
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
/// Windows: цветной значок приложения 32×32 — читается и на светлой, и на
/// тёмной панели задач (template-флага там нет, чёрный глиф на тёмной
/// панели почти не виден). Windows сам уменьшает его до 16–24 px.
#[cfg(not(target_os = "macos"))]
const TRAY_ICON: &[u8] = include_bytes!("../icons/32x32.png");

/// Windows: отпускание левой кнопки или двойной клик левой по значку в
/// трее открывает настройки. Двойной клик приходит как Click, DoubleClick и
/// снова Click — повторные вызовы show_settings безвредны.
fn tray_click_opens_settings(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } | TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        }
    )
}

/// Срок хранения истории соблюдается и когда приложение неделями не
/// перезапускают и ничего не диктуют: раз в час старые записи удаляются.
fn spawn_history_pruner(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
        let Some(state) = app.try_state::<AppShared>() else {
            continue;
        };
        let keep = store::read(&state.settings).history_keep.clone();
        store::prune_history(&app, &keep);
    });
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
            // Маркер для smoke-теста в CI: процесс дошёл до трея и островка.
            log::info!("startup complete: tray and island ready");
            let keep = store::read(&settings).history_keep.clone();
            store::prune_history(&handle, &keep);

            let active = resolve_startup_engine(&handle, &settings);
            let needs_download = matches!(active, StartupEngine::NeedsDownload(_));

            app.manage(AppShared {
                settings,
                model_status,
                worker_tx: Mutex::new(worker_tx.clone()),
                ctrl_tx: Mutex::new(hk.ctrl_tx),
                capture: hk.capture,
            });

            discovery::spawn_check(handle.clone());
            spawn_history_pruner(handle.clone());

            match active {
                // Первый запуск: ни одной модели на диске — качаем модель
                // по умолчанию (после скачивания worker загрузит её сам).
                StartupEngine::NeedsDownload(spec) => download_model(&handle, spec),
                StartupEngine::Ready => {
                    let _ = worker_tx.send(WorkerMsg::EnsureLoaded);
                }
            }

            // Без Accessibility/Input Monitoring ни хоткей, ни вставка
            // не работают — сразу показываем настройки с баннером. Первый
            // запуск (модели нет, идёт скачивание) — тоже: на Windows
            // разрешений нет, и без этого первый запуск был бы невидим
            // (значок в трее Windows 11 прячет в переполнение), пока в фоне
            // качаются сотни мегабайт.
            if !platform::accessibility_trusted()
                || !platform::input_monitoring_granted()
                || needs_download
            {
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

    /// Окно настроек на Windows: всегда тёмный нативный заголовок; Mica на
    /// Windows 11, непрозрачный фон на Windows 10; страница узнаёт вариант
    /// из ?backdrop=. Остальное (рамка, размер, скрытый старт) — из конфига.
    #[test]
    fn windows_settings_window_variants() {
        use super::{mica_supported, windows_settings_config};
        use tauri::utils::{config::Color, WindowEffect};
        use tauri::{Theme, WebviewUrl};
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let base = config
            .app
            .windows
            .iter()
            .find(|w| w.label == "settings")
            .expect("settings")
            .clone();

        let mica = windows_settings_config(base.clone(), true);
        assert_eq!(mica.theme, Some(Theme::Dark));
        assert!(mica.transparent);
        let fx = mica.window_effects.as_ref().expect("windowEffects");
        assert_eq!(fx.effects, vec![WindowEffect::MicaDark]);
        assert!(fx.state.is_none() && fx.radius.is_none() && fx.color.is_none());
        assert_eq!(mica.background_color, None);
        assert_eq!(mica.url, WebviewUrl::App("settings.html?backdrop=mica".into()));

        let opaque = windows_settings_config(base.clone(), false);
        assert_eq!(opaque.theme, Some(Theme::Dark));
        assert!(!opaque.transparent);
        assert!(opaque.window_effects.is_none());
        assert_eq!(opaque.background_color, Some(Color(26, 22, 43, 255)));
        assert_eq!(opaque.url, WebviewUrl::App("settings.html?backdrop=opaque".into()));

        for w in [&mica, &opaque] {
            // Нативный заголовок (без своего), окно скрыто до загрузки страницы.
            assert!(w.decorations && !w.create && !w.visible);
            assert_eq!((w.width, w.height), (base.width, base.height));
        }

        assert!(!mica_supported(19045)); // Windows 10 22H2
        assert!(!mica_supported(21999));
        assert!(mica_supported(22000)); // Windows 11 21H2
        assert!(mica_supported(26100)); // Windows 11 24H2
    }

    /// Уже созданное, но скрытое (грузится) окно не показывается раньше
    /// конца загрузки — кроме случая, когда загрузка зависла.
    #[test]
    fn settings_window_waits_for_page_load() {
        use super::{reveal_existing_settings, SETTINGS_LOAD_GRACE};
        use std::time::Duration;
        assert!(reveal_existing_settings(true, Duration::ZERO));
        assert!(!reveal_existing_settings(false, Duration::from_millis(300)));
        assert!(reveal_existing_settings(false, SETTINGS_LOAD_GRACE));
        assert!(reveal_existing_settings(false, Duration::MAX));
    }

    /// Windows: настройки открывает левый клик (отпускание) и двойной клик
    /// левой; правая кнопка — меню, наведение — ничего.
    #[test]
    fn tray_left_click_opens_settings() {
        use super::tray_click_opens_settings;
        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent, TrayIconId};
        let id = || TrayIconId::new("main");
        let position = tauri::PhysicalPosition::new(0.0, 0.0);
        let click = |button, button_state| TrayIconEvent::Click {
            id: id(),
            position,
            rect: tauri::Rect::default(),
            button,
            button_state,
        };
        let double = |button| TrayIconEvent::DoubleClick {
            id: id(),
            position,
            rect: tauri::Rect::default(),
            button,
        };
        assert!(tray_click_opens_settings(&click(MouseButton::Left, MouseButtonState::Up)));
        assert!(!tray_click_opens_settings(&click(MouseButton::Left, MouseButtonState::Down)));
        assert!(!tray_click_opens_settings(&click(MouseButton::Right, MouseButtonState::Up)));
        assert!(tray_click_opens_settings(&double(MouseButton::Left)));
        assert!(!tray_click_opens_settings(&double(MouseButton::Right)));
        assert!(!tray_click_opens_settings(&TrayIconEvent::Enter {
            id: id(),
            position,
            rect: tauri::Rect::default(),
        }));
    }

    /// Лог на Windows лежит в папке приложения; её имя продублировано в
    /// lib.rs (логирование стартует до Tauri) и должно совпадать с конфигом.
    #[test]
    fn log_folder_matches_identifier() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(super::APP_IDENTIFIER, config.identifier);
    }

    /// Установщик Windows: значок .ico (ресурс exe и NSIS), издатель,
    /// английский и русский языки, WebView2 через загрузчик.
    #[test]
    fn windows_bundle_is_configured() {
        use std::path::Path;
        use tauri::utils::config::WebviewInstallMode;
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let bundle = &config.bundle;
        assert!(bundle.icon.iter().any(|i| i == "icons/icon.ico"), "{:?}", bundle.icon);
        assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.ico").is_file());
        assert_eq!(bundle.publisher.as_deref(), Some("DIZRAID"));
        let nsis = bundle.windows.nsis.as_ref().expect("bundle.windows.nsis");
        let ico = Some(Path::new("icons/icon.ico"));
        assert_eq!(nsis.installer_icon.as_deref(), ico);
        assert_eq!(nsis.uninstaller_icon.as_deref(), ico);
        // Картинки мастера установки (src-tauri/windows/installer-art.py).
        for image in [&nsis.sidebar_image, &nsis.header_image] {
            let image = image.as_deref().expect("sidebarImage / headerImage");
            assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join(image).is_file(), "{image:?}");
        }
        let languages = nsis.languages.as_deref().unwrap_or_default();
        for lang in ["English", "Russian"] {
            assert!(languages.iter().any(|l| l == lang), "нет языка {lang}: {languages:?}");
        }
        assert!(matches!(
            bundle.windows.webview_install_mode,
            WebviewInstallMode::DownloadBootstrapper { .. }
        ));
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

    /// Каждое окно получает только нужные ему команды (build.rs включает
    /// проверку ACL для команд приложения). Островок — только события,
    /// геометрию выреза и отмену записи; окно настроек — остальное.
    #[test]
    fn ipc_is_scoped_per_window() {
        use crate::app_commands::APP_COMMANDS;
        use std::collections::BTreeSet;

        // Никаких других файлов capabilities (например, прежнего default.json
        // с core:default для обоих окон).
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let files: BTreeSet<String> = std::fs::read_dir(dir)
            .expect("capabilities/")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, BTreeSet::from(["island.json".into(), "settings.json".into()]));

        let parse = |s: &str| -> (Vec<String>, BTreeSet<String>) {
            let v: serde_json::Value = serde_json::from_str(s).expect("capability json");
            let list = |k: &str| -> Vec<String> {
                v[k].as_array()
                    .expect(k)
                    .iter()
                    .map(|p| p.as_str().expect("string").to_string())
                    .collect()
            };
            (list("windows"), list("permissions").into_iter().collect())
        };
        let (island_windows, island) = parse(include_str!("../capabilities/island.json"));
        let (settings_windows, settings) = parse(include_str!("../capabilities/settings.json"));
        assert_eq!(island_windows, ["island"]);
        assert_eq!(settings_windows, ["settings"]);

        let expected_island: BTreeSet<String> = [
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "allow-island-metrics",
            "allow-cancel-recording",
        ]
        .map(String::from)
        .into();
        assert_eq!(island, expected_island);

        // Каждая команда приложения разрешена ровно одному окну, и
        // разрешений на несуществующие команды нет.
        let allow = |c: &str| format!("allow-{}", c.replace('_', "-"));
        for cmd in APP_COMMANDS {
            let p = allow(cmd);
            let n = island.contains(&p) as u8 + settings.contains(&p) as u8;
            assert_eq!(n, 1, "{cmd}: разрешена {n} окнам");
        }
        let known: BTreeSet<String> = APP_COMMANDS.iter().map(|c| allow(c)).collect();
        for p in island.iter().chain(&settings) {
            assert!(
                p.starts_with("core:") || known.contains(p),
                "{p}: нет такой команды в app_commands.rs"
            );
        }
        // Из core — только то, что реально зовут страницы (event.listen,
        // app.getVersion, перетаскивание окна), без наборов вроде core:default.
        for p in island.iter().chain(&settings).filter(|p| p.starts_with("core:")) {
            assert!(p.contains(":allow-"), "{p}: только отдельные allow-разрешения");
        }

        // Список для ACL совпадает с generate_handler!: команда, которой нет
        // в списке, была бы отклонена в любом окне.
        let src = include_str!("lib.rs");
        let start = src.find("generate_handler![").expect("generate_handler!") + 18;
        let end = start + src[start..].find(']').expect("]");
        let handlers: BTreeSet<&str> = src[start..end]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let listed: BTreeSet<&str> = APP_COMMANDS.iter().copied().collect();
        assert_eq!(handlers, listed);
    }

    /// Строгая CSP: без inline-скриптов и стилей, без внешних адресов;
    /// IPC ходит через ipc: (macOS) и http://ipc.localhost (Windows).
    #[test]
    fn csp_is_strict() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let sec = &config.app.security;
        let csp = sec.csp.as_ref().expect("security.csp задана").to_string();
        for d in [
            "default-src 'none'",
            "script-src 'self'",
            "style-src 'self'",
            "connect-src ipc: http://ipc.localhost",
            "object-src 'none'",
            "base-uri 'none'",
            "form-action 'none'",
            "frame-ancestors 'none'",
        ] {
            assert!(csp.contains(d), "в CSP нет «{d}»: {csp}");
        }
        for bad in ["unsafe-inline", "unsafe-eval", "http:", "https:", "data:", "blob:", "*"] {
            let without_ipc = csp.replace("http://ipc.localhost", "");
            assert!(!without_ipc.contains(bad), "в CSP есть «{bad}»: {csp}");
        }
        assert!(sec.freeze_prototype);
        // Tauri добавляет в script-src хеши inline-скриптов, только если
        // ему разрешено менять CSP; inline-скриптов в ui/ и так нет.
        assert!(matches!(
            sec.dangerous_disable_asset_csp_modification,
            tauri::utils::config::DisabledCspModificationKind::Flag(false)
        ));
        for page in [include_str!("../../ui/settings.html"), include_str!("../../ui/island.html")] {
            assert!(!page.contains("<script>"), "inline-скрипт в ui/*.html");
            assert!(!page.contains(" style="), "атрибут style в ui/*.html");
        }
    }

    /// Статические библиотеки sherpa-onnx (с ONNX Runtime) входят в каждый
    /// бинарник, а релиз, откуда их берёт сборка, можно перезаписать. CI и
    /// release.yml сверяют архив с sherpa-onnx.sha256: сменилась версия
    /// sherpa-onnx-sys в Cargo.lock — нужны и новые суммы.
    #[test]
    fn sherpa_onnx_archives_are_pinned() {
        let lock = include_str!("../Cargo.lock");
        let key = "name = \"sherpa-onnx-sys\"\nversion = \"";
        let rest = &lock[lock.find(key).expect("sherpa-onnx-sys в Cargo.lock") + key.len()..];
        let version = &rest[..rest.find('"').expect("версия")];
        let pins = include_str!("../sherpa-onnx.sha256");
        for platform in ["osx-arm64-static-lib", "win-x64-static-MT-Release-lib"] {
            let name = format!("sherpa-onnx-v{version}-{platform}.tar.bz2");
            let line = pins
                .lines()
                .find(|l| !l.starts_with('#') && l.ends_with(&format!("  {name}")))
                .unwrap_or_else(|| panic!("нет SHA-256 для {name} в sherpa-onnx.sha256"));
            let hash = line.split("  ").next().unwrap_or_default();
            assert!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "{line}"
            );
        }
    }

    #[test]
    fn open_url_allowlist() {
        use super::url::allowed;
        for ok in [
            "https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models",
            "https://huggingface.co/nvidia/parakeet-tdt_ctc-110m",
            "https://platform.openai.com/api-keys",
            "https://console.groq.com/keys",
            "https://elevenlabs.io/app/settings/api-keys",
            "https://GitHub.com/DIZRAID/voxy?tab=readme#install",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in [
            "http://github.com/",
            "https://evil.com/",
            "https://github.com.evil.com/",
            "https://github.com@evil.com/",
            "https://evil.com\\@github.com/",
            "https://github.com:443/",
            "https://github.com./",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https://github.com/a b",
            "https://github.com/a,b",
            "https://github.com/\"x",
            "https://github.com/\nx",
            "https://github.com/\u{200B}",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
        assert!(!allowed(&format!("https://github.com/{}", "a".repeat(2100))));
    }
}
