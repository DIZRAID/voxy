//! «Островок» у выреза камеры: прозрачная неактивирующаяся NSPanel поверх
//! строки меню. Окно всегда живо и прозрачно для кликов; вся анимация — CSS.

use serde_json::json;
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
            .map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!("панель островка: {e}").into()))?;

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

/// Порт на Windows: обычное прозрачное always-on-top окно без NSPanel.
#[cfg(not(target_os = "macos"))]
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    let window = WebviewWindowBuilder::new(app, "island", WebviewUrl::App("island.html".into()))
        .title("Voxy Island")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .visible_on_all_workspaces(true)
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        let mw = monitor.size().width as f64 / scale;
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition {
            x: (mw - WIDTH) / 2.0,
            y: 0.0,
        }));
    }
    Ok(())
}

/// Состояния: recording | processing | done | error | cancelled
pub fn set_state(app: &AppHandle, state: &str, message: Option<String>) {
    let _ = app.emit_to("island", "state", json!({ "state": state, "message": message }));
}

/// Начало записи с лимитом — островок подсвечивает таймер у границы.
pub fn set_recording(app: &AppHandle, max_s: u64) {
    let _ = app.emit_to("island", "state", json!({ "state": "recording", "max_s": max_s }));
}

/// Кликабельность островка: во время записи он принимает клики
/// (кнопка Cancel), в остальное время прозрачен для мыши, чтобы
/// не блокировать строку меню под собой.
pub fn set_interactive(app: &AppHandle, interactive: bool) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.set_ignore_cursor_events(!interactive);
    }
}
