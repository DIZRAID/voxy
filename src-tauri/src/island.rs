//! «Островок» — индикатор записи. Окно создаётся один раз при запуске,
//! всегда живо и прозрачно для кликов (кроме записи — кнопка Cancel); вся
//! анимация — CSS в island.html.
//!
//! - macOS: пилюля у выреза камеры. Прозрачная неактивирующаяся NSPanel
//!   поверх строки меню, на всех Spaces и поверх полноэкранных приложений.
//! - Windows: выреза нет, поэтому пилюля плавает внизу по центру, над
//!   панелью задач, как системный индикатор громкости/яркости Windows 11.
//!   Обычное прозрачное always-on-top окно без рамки и тени; низ окна стоит
//!   ровно на нижней границе рабочей области основного монитора, отступ над
//!   панелью задач рисует страница (island.css, html[data-platform="win"]).
//!   Окно скрыто до загрузки страницы и никогда не забирает фокус.
//!
//! Что должно быть на экране, Rust хранит сам (SHOWN): каждое событие
//! `state` несёт номер `seq`, страница подтверждает его командой
//! island_sync, а (пере)загрузившаяся страница той же командой забирает
//! текущее состояние. На Windows в начале записи окно заново поднимается
//! поверх всех, а сторож (watchdog) ждёт подтверждения: нет ответа —
//! страница перезагружается, всё ещё нет — окно пересоздаётся. Так
//! островок переживает падение процесса WebView2 (сон, смена дисплея или
//! драйвера), другие окна «поверх всех» и скрытие окна, которые раньше
//! гасили его до перезапуска Voxy.

use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

pub const WIDTH: f64 = 520.0;
pub const HEIGHT: f64 = 150.0;

/// Реальная геометрия выреза камеры. Меряется один раз при старте
/// (главный поток) и отдаётся island.js командой island_metrics —
/// пилюля в покое рисуется РОВНО по физическому вырезу, а не «на глаз».
#[derive(Clone, Copy, serde::Serialize)]
pub struct IslandMetrics {
    pub has_notch: bool,
    /// Ширина выреза (логические пункты).
    pub notch_w: f64,
    /// Высота выреза (= safeAreaInsets.top).
    pub notch_h: f64,
    /// Высота строки меню — высота пилюли на экранах без выреза.
    pub bar_h: f64,
}

impl Default for IslandMetrics {
    fn default() -> Self {
        Self {
            has_notch: false,
            notch_w: 190.0,
            notch_h: 34.0,
            bar_h: 36.0,
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{HEIGHT, WIDTH};
    use tauri::{AppHandle, Manager, WebviewUrl};
    use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

    tauri_panel! {
        panel!(IslandPanel {
            config: {
                can_become_key_window: false,
                is_floating_panel: true
            }
        })
    }

    /// Замер физического выреза. Только главный поток (NSScreen).
    pub fn measure_metrics() -> super::IslandMetrics {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSScreen;

        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("measure_metrics вне главного потока — беру дефолты");
            return super::IslandMetrics::default();
        };
        let Some(screen) = NSScreen::mainScreen(mtm) else {
            return super::IslandMetrics::default();
        };

        let frame = screen.frame();
        let visible = screen.visibleFrame();
        // строка меню: зазор между верхом экрана и верхом видимой области
        let bar_h = (frame.origin.y + frame.size.height)
            - (visible.origin.y + visible.size.height);
        let notch_h = screen.safeAreaInsets().top;

        if notch_h > 0.0 {
            let left = screen.auxiliaryTopLeftArea();
            let right = screen.auxiliaryTopRightArea();
            let notch_w = frame.size.width - left.size.width - right.size.width;
            if notch_w > 0.0 && notch_w < frame.size.width {
                log::info!("вырез: {notch_w:.0}×{notch_h:.0}, строка меню {bar_h:.0}");
                return super::IslandMetrics {
                    has_notch: true,
                    notch_w,
                    notch_h,
                    bar_h: bar_h.max(notch_h),
                };
            }
        }
        log::info!("экран без выреза, строка меню {bar_h:.0}");
        super::IslandMetrics {
            has_notch: false,
            bar_h: if bar_h > 0.0 { bar_h } else { 24.0 },
            ..super::IslandMetrics::default()
        }
    }

    pub fn create(app: &AppHandle) -> tauri::Result<()> {
        let (x, y) = position(app);

        let panel = PanelBuilder::<_, IslandPanel>::new(app, "island")
            .url(WebviewUrl::App("island.html".into()))
            .title("Voxy Island")
            .position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: WIDTH,
                height: HEIGHT,
            }))
            .level(PanelLevel::Status) // 25 — выше строки меню (24)
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .style_mask(StyleMask::empty().borderless().nonactivating_panel())
            .with_window(|w| w.decorations(false).transparent(true).focusable(false))
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary()
                    .stationary(),
            )
            .build()
            .map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!("панель островка: {e}")))?;

        // Островок чисто визуальный: пропускаем клики насквозь, чтобы
        // не блокировать меню приложений под прозрачной областью окна.
        if let Some(window) = app.get_webview_window("island") {
            let _ = window.set_ignore_cursor_events(true);
        }
        panel.show();
        Ok(())
    }

    fn position(app: &AppHandle) -> (f64, f64) {
        if let Ok(Some(monitor)) = app.primary_monitor() {
            let scale = monitor.scale_factor();
            let mw = monitor.size().width as f64 / scale;
            let mx = monitor.position().x as f64 / scale;
            return (mx + (mw - WIDTH) / 2.0, 0.0);
        }
        (0.0, 0.0)
    }
}

#[cfg(target_os = "macos")]
pub use macos::create;
#[cfg(target_os = "macos")]
pub use macos::measure_metrics;

/// Windows/прочее: выреза нет, пилюля в покое скрыта; активные
/// состояния используют дефолтную высоту.
#[cfg(not(target_os = "macos"))]
pub fn measure_metrics() -> IslandMetrics {
    IslandMetrics::default()
}

/// Windows: обычное прозрачное always-on-top окно (NSPanel нет), внизу по
/// центру над панелью задач (см. doc модуля и bottom_center).
#[cfg(not(target_os = "macos"))]
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    use tauri::webview::PageLoadEvent;
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let window = WebviewWindowBuilder::new(app, "island", WebviewUrl::App("island.html".into()))
        .title("Voxy Island")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // WS_EX_NOACTIVATE: клик по островку (Cancel) не активирует окно.
        .focusable(false)
        // Первый show() идёт с SW_SHOWNOACTIVATE (tao: MARKER_DONT_FOCUS),
        // и WebView2 тоже не берёт фокус: запуск Voxy, в том числе
        // автозапуск при входе, не отнимает фокус у окна, где человек печатает.
        .focused(false)
        // Скрыто до конца загрузки страницы: иначе WebView2 сначала рисует
        // свой фон, и при каждом запуске мелькал бы прямоугольник 520×150.
        .visible(false)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.show();
            }
        })
        // visible_on_all_workspaces не ставим: в tao на Windows это no-op
        // (виртуальные рабочие столы Windows он не поддерживает).
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    // Новое окно (в том числе пересозданное сторожем) стоит не на месте.
    *PLACED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // До show(): окно ещё скрыто, перенос не виден.
    reposition(app);
    Ok(())
}

/// Прямоугольник окна островка в физических пикселях.
#[cfg(any(test, not(target_os = "macos")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// Windows: окно WIDTH×HEIGHT логических точек по центру рабочей области,
/// низ окна ровно на её нижней границе. Рабочая область монитора уже не
/// включает панель задач, где бы та ни стояла: при панели снизу низ окна
/// касается её верхнего края; при панели сбоку, сверху или автоскрытии —
/// низа экрана. Всё в физических пикселях: так не важно, с каким DPI окно
/// было создано (логические координаты tao пересчитал бы по DPI монитора,
/// на котором окно стоит сейчас).
#[cfg(any(test, not(target_os = "macos")))]
fn bottom_center(wa_x: i32, wa_y: i32, wa_width: u32, wa_height: u32, scale: f64) -> Placement {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let width = (WIDTH * scale).round() as u32;
    let height = (HEIGHT * scale).round() as u32;
    Placement {
        x: wa_x + ((wa_width as f64 - WIDTH * scale) / 2.0).round() as i32,
        y: wa_y + wa_height as i32 - height as i32,
        width,
        height,
    }
}

/// Ставит островок по bottom_center для основного монитора, если место
/// изменилось с прошлого раза. Зовётся при создании и в начале каждой
/// записи (set_recording): так островок догоняет перенесённую или
/// изменённую панель задач, смену разрешения, масштаба или основного
/// монитора без подписки на WM_SETTINGCHANGE/WM_DISPLAYCHANGE. В покое
/// пилюля на Windows скрыта, поэтому перенос в начале записи не виден.
/// primary_monitor() — прямой вызов Win32 (MonitorFromPoint), без похода
/// на главный поток; set_size/set_position с чужого потока только
/// ставятся в очередь главного.
/// Где окно стоит сейчас (последний перенос); None — окно новое.
#[cfg(not(target_os = "macos"))]
static PLACED: Mutex<Option<Placement>> = Mutex::new(None);

#[cfg(not(target_os = "macos"))]
fn reposition(app: &AppHandle) {
    use tauri::Manager;

    let Some(window) = app.get_webview_window("island") else {
        return;
    };
    let monitor = match app.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => {
            log::warn!("островок: основной монитор не найден, позиция по умолчанию");
            return;
        }
        Err(e) => {
            log::warn!("островок: основной монитор: {e}");
            return;
        }
    };
    let wa = monitor.work_area();
    let scale = monitor.scale_factor();
    let p = bottom_center(
        wa.position.x,
        wa.position.y,
        wa.size.width,
        wa.size.height,
        scale,
    );

    let mut placed = PLACED.lock().unwrap_or_else(|e| e.into_inner());
    if *placed == Some(p) {
        return;
    }
    // Дважды: если окно стоит на мониторе с другим DPI, первый перенос
    // вызывает WM_DPICHANGED, tao сохраняет логический размер и сдвигает
    // окно к предложенному Windows прямоугольнику; второй проход ставит
    // точно (DPI уже совпадает, сообщения больше не будет).
    for _ in 0..2 {
        let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: p.width,
            height: p.height,
        }));
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: p.x,
            y: p.y,
        }));
    }
    *placed = Some(p);
    log::info!(
        "островок: {}×{} в ({}, {}), масштаб {scale}, рабочая область {}×{} в ({}, {})",
        p.width,
        p.height,
        p.x,
        p.y,
        wa.size.width,
        wa.size.height,
        wa.position.x,
        wa.position.y
    );
}

/// Последнее состояние, отправленное островку.
struct Shown {
    state: String,
    message: Option<String>,
    max_s: u64,
    since: Instant,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);
/// Номер последнего отправленного события `state`.
static SENT: AtomicU64 = AtomicU64::new(0);
/// Наибольший номер, который страница островка подтвердила.
static ACKED: AtomicU64 = AtomicU64::new(0);

fn publish(app: &AppHandle, state: &str, message: Option<String>, max_s: u64) -> u64 {
    let seq = {
        let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        *shown = Some(Shown {
            state: state.to_string(),
            message: message.clone(),
            max_s,
            since: Instant::now(),
        });
        // Номер выдаётся под тем же локом: порядок номеров = порядок состояний.
        SENT.fetch_add(1, Ordering::SeqCst) + 1
    };
    let _ = app.emit_to(
        "island",
        "state",
        json!({ "state": state, "message": message, "max_s": max_s, "seq": seq }),
    );
    seq
}

/// Команда island_sync. `ack` — номер события, которое страница только что
/// показала. Без `ack` страница только что загрузилась и берёт текущее
/// состояние: после ответа она показывает всё, что было отправлено.
pub fn sync(ack: Option<u64>) -> serde_json::Value {
    let shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    let seq = SENT.load(Ordering::SeqCst);
    ACKED.fetch_max(ack.unwrap_or(seq).min(seq), Ordering::SeqCst);
    match shown.as_ref() {
        Some(s) => json!({
            "state": s.state,
            "message": s.message,
            "max_s": s.max_s,
            "elapsed_ms": s.since.elapsed().as_millis() as u64,
            "seq": seq,
        }),
        None => json!({ "state": "idle", "seq": seq }),
    }
}

/// Состояния: recording | processing | done | error | cancelled
pub fn set_state(app: &AppHandle, state: &str, message: Option<String>) {
    publish(app, state, message, 0);
}

/// Начало записи с лимитом — островок подсвечивает таймер у границы.
pub fn set_recording(app: &AppHandle, max_s: u64) {
    // Windows: до появления пилюли — на случай, если панель задач или
    // экран изменились с прошлой записи (дёшево, если ничего не менялось),
    // и окно снова поверх всех и видимо.
    #[cfg(not(target_os = "macos"))]
    {
        reposition(app);
        raise(app);
    }
    let seq = publish(app, "recording", None, max_s);
    #[cfg(not(target_os = "macos"))]
    spawn_watchdog(app.clone(), seq);
    #[cfg(target_os = "macos")]
    let _ = seq;
}

/// Сколько ждать подтверждения от страницы, прежде чем её лечить.
#[cfg(not(target_os = "macos"))]
const ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2000);

/// Что делать сторожу на этом шаге: страница подтвердила событие `seq`
/// (или более позднее) — ничего; иначе первая попытка — перезагрузить
/// страницу, вторая — пересоздать окно.
#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum Repair {
    None,
    Reload,
    Recreate,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn repair_step(acked: u64, seq: u64, attempt: u32) -> Repair {
    if acked >= seq {
        Repair::None
    } else if attempt == 0 {
        Repair::Reload
    } else {
        Repair::Recreate
    }
}

/// Windows: сторож одной записи. Поток спит, не трогая главный поток, и
/// лечит островок, только если страница не подтвердила начало записи.
#[cfg(not(target_os = "macos"))]
fn spawn_watchdog(app: AppHandle, seq: u64) {
    let spawned = std::thread::Builder::new()
        .name("voxy-island-watchdog".into())
        .spawn(move || {
            for attempt in 0..2 {
                std::thread::sleep(ACK_TIMEOUT);
                match repair_step(ACKED.load(Ordering::SeqCst), seq, attempt) {
                    Repair::None => return,
                    Repair::Reload => {
                        log::warn!("островок не ответил на начало записи — перезагружаю страницу");
                        reload(&app);
                    }
                    Repair::Recreate => {
                        log::warn!("островок не ожил после перезагрузки — пересоздаю окно");
                        recreate(&app);
                    }
                }
            }
            std::thread::sleep(ACK_TIMEOUT);
            if ACKED.load(Ordering::SeqCst) < seq {
                log::error!("островок не отвечает и после пересоздания окна");
            } else {
                log::info!("островок восстановлен");
            }
        });
    if let Err(e) = spawned {
        log::warn!("островок: сторож не запущен: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn reload(app: &AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("island") {
        if let Err(e) = window.reload() {
            log::warn!("островок: перезагрузка страницы: {e}");
        }
    }
}

/// Закрыть окно островка и создать заново. Вызывается со стороннего потока
/// (сторожа): build() главный поток не блокирует.
#[cfg(not(target_os = "macos"))]
fn recreate(app: &AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("island") {
        if let Err(e) = window.destroy() {
            log::warn!("островок: закрытие окна: {e}");
        }
        // destroy() выполняется на главном потоке; ярлык «island» свободен,
        // только когда окно действительно закрыто.
        for _ in 0..50 {
            if app.get_webview_window("island").is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    if let Err(e) = create(app) {
        log::error!("островок: не удалось пересоздать окно: {e}");
    }
}

/// Windows: окно снова наверху полосы «поверх всех» и видимо. Другие
/// topmost-окна, открытые позже, оказываются выше островка, а само окно
/// могло быть скрыто; SetWindowPos это исправляет, не активируя окно.
/// SWP_ASYNCWINDOWPOS: запрос ставится в очередь главного потока, поток
/// записи не ждёт его.
#[cfg(target_os = "windows")]
fn raise(app: &AppHandle) {
    use tauri::Manager;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW,
    };
    let Some(window) = app.get_webview_window("island") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    // SAFETY: hwnd — действующее окно островка (его ещё держит window);
    // флаги не меняют размер, место и фокус, только z-порядок и видимость.
    let ok = unsafe {
        SetWindowPos(
            hwnd.0 as _,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_ASYNCWINDOWPOS,
        )
    };
    if ok == 0 {
        log::warn!("островок: SetWindowPos не удался");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn raise(_app: &AppHandle) {}

/// Кликабельность островка: во время записи он принимает клики
/// (кнопка Cancel), в остальное время прозрачен для мыши, чтобы
/// не блокировать то, что под ним (строку меню на macOS, низ окон над
/// панелью задач на Windows).
pub fn set_interactive(app: &AppHandle, interactive: bool) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.set_ignore_cursor_events(!interactive);
    }
}

#[cfg(test)]
mod tests {
    use super::{bottom_center, repair_step, Placement, Repair};

    /// Сторож лечит только неподтверждённое событие: сначала перезагрузка
    /// страницы, потом пересоздание окна. Подтверждение более позднего
    /// события (островок уже показал следующее состояние) тоже считается.
    #[test]
    fn watchdog_escalates_only_without_ack() {
        assert_eq!(repair_step(7, 7, 0), Repair::None);
        assert_eq!(repair_step(9, 7, 1), Repair::None);
        assert_eq!(repair_step(6, 7, 0), Repair::Reload);
        assert_eq!(repair_step(6, 7, 1), Repair::Recreate);
    }

    /// Низ окна — ровно на нижней границе рабочей области, по центру;
    /// размер — WIDTH×HEIGHT логических точек в физических пикселях.
    #[test]
    fn island_sits_bottom_center_of_work_area() {
        // 1920×1080, 100 %, панель задач снизу 48 px.
        assert_eq!(
            bottom_center(0, 0, 1920, 1032, 1.0),
            Placement { x: 700, y: 882, width: 520, height: 150 }
        );
        // 2880×1620, 150 %, панель снизу 72 px: низ окна = 1548.
        let p = bottom_center(0, 0, 2880, 1548, 1.5);
        assert_eq!(p, Placement { x: 1050, y: 1323, width: 780, height: 225 });
        assert_eq!(p.y + p.height as i32, 1548);
        // 125 %: дробные размеры округляются, низ всё равно точный.
        let p = bottom_center(0, 0, 2560, 1380, 1.25);
        assert_eq!((p.width, p.height), (650, 188));
        assert_eq!(p.y + p.height as i32, 1380);
        // Панель задач слева (рабочая область сдвинута) — центр рабочей
        // области, низ экрана.
        assert_eq!(
            bottom_center(62, 0, 1858, 1080, 1.0),
            Placement { x: 731, y: 930, width: 520, height: 150 }
        );
        // Панель сверху: рабочая область начинается ниже, низ — низ экрана.
        assert_eq!(bottom_center(0, 48, 1920, 1032, 1.0).y, 930);
        // Отрицательные координаты: формула не привязана к (0, 0).
        assert_eq!(
            bottom_center(-1920, 0, 1920, 1032, 1.0),
            Placement { x: -1220, y: 882, width: 520, height: 150 }
        );
        // Мусорный масштаб — как 100 %.
        assert_eq!(bottom_center(0, 0, 1920, 1032, 0.0), bottom_center(0, 0, 1920, 1032, 1.0));
        assert_eq!(
            bottom_center(0, 0, 1920, 1032, f64::NAN),
            bottom_center(0, 0, 1920, 1032, 1.0)
        );
    }
}
