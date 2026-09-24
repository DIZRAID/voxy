use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tauri::{AppHandle, Emitter, Manager};

pub const HISTORY_LIMIT: usize = 50;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordMode {
    Ptt,
    Toggle,
    /// Гибрид: быстрый тап (<1 c) — запись до повторного нажатия,
    /// удержание ≥1 c — классический push-to-talk.
    Dynamic,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Имя клавиши, например "MetaRight" (таблица в hotkey.rs).
    pub hotkey: String,
    pub mode: RecordMode,
    pub min_duration_ms: u64,
    /// Лимит записи. Поле переименовано из max_duration_s (жёсткие 30 c
    /// обрывали речь) — старое значение из settings.json сознательно
    /// отбрасывается в пользу нового default.
    pub max_record_s: u64,
    pub sounds: bool,
    /// None — системный микрофон по умолчанию.
    pub mic_device: Option<String>,
    pub autostart: bool,
    /// Срок хранения истории: "24h" | "7d" | "30d" | "forever".
    pub history_keep: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey().to_string(),
            mode: RecordMode::Ptt,
            min_duration_ms: 300,
            max_record_s: 120,
            sounds: true,
            mic_device: None,
            autostart: false,
            history_keep: "7d".to_string(),
        }
    }
}

/// Срок хранения в миллисекундах; None — хранить вечно.
pub fn retention_ms(keep: &str) -> Option<u64> {
    match keep {
        "24h" => Some(24 * 3600 * 1000),
        "7d" => Some(7 * 24 * 3600 * 1000),
        "30d" => Some(30 * 24 * 3600 * 1000),
        _ => None,
    }
}

pub fn default_hotkey() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "MetaRight"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "ControlRight"
    }
}

pub type SharedSettings = Arc<RwLock<Settings>>;

/// Чтение настроек, переживающее «отравленный» лок: паника в одном потоке
/// не должна навсегда ломать хоткей и распознавание в остальных.
pub fn read(s: &SharedSettings) -> RwLockReadGuard<'_, Settings> {
    s.read().unwrap_or_else(|e| e.into_inner())
}

pub fn write(s: &SharedSettings) -> RwLockWriteGuard<'_, Settings> {
    s.write().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryEntry {
    pub text: String,
    pub ts_ms: u64,
    pub duration_ms: u64,
}

pub fn data_dir(app: &AppHandle) -> PathBuf {
    let dir = app.path().app_data_dir().unwrap_or_else(|e| {
        log::error!("app data dir недоступен ({e}), использую временную папку");
        std::env::temp_dir().join("com.dizraid.voice")
    });
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Атомарная запись: сначала во временный файл, потом rename. Сбой или
/// выключение посреди записи не оставит обрезанный JSON, который при
/// следующем запуске молча превратился бы в настройки по умолчанию.
fn write_atomic(path: &PathBuf, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

fn settings_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("settings.json")
}

fn history_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("history.json")
}

pub fn load_settings(app: &AppHandle) -> Settings {
    fs::read_to_string(settings_path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(app: &AppHandle, settings: &Settings) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        if let Err(e) = write_atomic(&settings_path(app), &json) {
            log::error!("не удалось сохранить настройки: {e}");
        }
    }
}

pub fn load_history(app: &AppHandle) -> Vec<HistoryEntry> {
    fs::read_to_string(history_path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(app: &AppHandle, history: &[HistoryEntry]) {
    if let Ok(json) = serde_json::to_string_pretty(history) {
        if let Err(e) = write_atomic(&history_path(app), &json) {
            log::error!("не удалось сохранить историю: {e}");
        }
    }
}

pub fn push_history(app: &AppHandle, entry: HistoryEntry, keep: &str) {
    let mut history = load_history(app);
    history.insert(0, entry);
    history.truncate(HISTORY_LIMIT);
    apply_retention(&mut history, keep);
    save_history(app, &history);
    let _ = app.emit_to("settings", "history-updated", ());
}

/// Удаляет записи старше срока хранения. Вызывается при запуске,
/// при смене срока и при каждом добавлении.
pub fn prune_history(app: &AppHandle, keep: &str) {
    let mut history = load_history(app);
    let before = history.len();
    apply_retention(&mut history, keep);
    if history.len() != before {
        save_history(app, &history);
        let _ = app.emit_to("settings", "history-updated", ());
    }
}

fn apply_retention(history: &mut Vec<HistoryEntry>, keep: &str) {
    if let Some(ttl) = retention_ms(keep) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        history.retain(|e| now.saturating_sub(e.ts_ms) < ttl);
    }
}

pub fn clear_history(app: &AppHandle) {
    save_history(app, &[]);
    let _ = app.emit_to("settings", "history-updated", ());
}
