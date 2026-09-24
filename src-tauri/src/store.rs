use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
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
    /// Активный движок: id локальной модели из каталога или
    /// "online:<provider>" (openai | groq | elevenlabs).
    pub active_model: String,
    /// Через сколько минут простоя выгружать локальную модель из памяти
    /// (0 — никогда). На 8-гигабайтном Mac это ~1 ГБ, возвращаемый системе.
    pub unload_after_min: u32,
}

pub const ONLINE_PREFIX: &str = "online:";

impl Settings {
    /// Провайдер, если активен онлайн-движок.
    pub fn online_provider(&self) -> Option<&str> {
        self.active_model.strip_prefix(ONLINE_PREFIX)
    }
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
            active_model: crate::models::DEFAULT_MODEL.to_string(),
            unload_after_min: 5,
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
/// Имя временного файла своё у каждой записи: две записи из разных потоков
/// не пишут в один и тот же .tmp.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}-{seq}.tmp", std::process::id()));
    let result = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
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

/// Все чтения-изменения-записи history.json идут под этим локом: иначе
/// очистка истории, пришедшая между чтением и записью в push_history,
/// была бы молча отменена (вернулись бы старые записи).
fn history_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_history_file(app: &AppHandle) -> Vec<HistoryEntry> {
    fs::read_to_string(history_path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// История для показа. Записи старше срока хранения не возвращаются и
/// удаляются с диска.
pub fn load_history(app: &AppHandle, keep: &str) -> Vec<HistoryEntry> {
    let _guard = history_lock();
    let mut history = read_history_file(app);
    if apply_retention(&mut history, keep) {
        save_history(app, &history);
    }
    history
}

fn save_history(app: &AppHandle, history: &[HistoryEntry]) {
    if let Ok(json) = serde_json::to_string_pretty(history) {
        if let Err(e) = write_atomic(&history_path(app), &json) {
            log::error!("не удалось сохранить историю: {e}");
        }
    }
}

pub fn push_history(app: &AppHandle, entry: HistoryEntry, keep: &str) {
    {
        let _guard = history_lock();
        let mut history = read_history_file(app);
        history.insert(0, entry);
        history.truncate(HISTORY_LIMIT);
        apply_retention(&mut history, keep);
        save_history(app, &history);
    }
    let _ = app.emit_to("settings", "history-updated", ());
}

/// Удаляет записи старше срока хранения. Вызывается при запуске, при
/// смене срока, раз в час (lib.rs) и при каждом добавлении и чтении.
pub fn prune_history(app: &AppHandle, keep: &str) {
    let changed = {
        let _guard = history_lock();
        let mut history = read_history_file(app);
        let changed = apply_retention(&mut history, keep);
        if changed {
            save_history(app, &history);
        }
        changed
    };
    if changed {
        let _ = app.emit_to("settings", "history-updated", ());
    }
}

/// Убирает записи старше срока; true, если что-то убрано.
fn apply_retention(history: &mut Vec<HistoryEntry>, keep: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    retain_recent(history, keep, now)
}

fn retain_recent(history: &mut Vec<HistoryEntry>, keep: &str, now_ms: u64) -> bool {
    let before = history.len();
    if let Some(ttl) = retention_ms(keep) {
        history.retain(|e| now_ms.saturating_sub(e.ts_ms) < ttl);
    }
    history.len() != before
}

pub fn clear_history(app: &AppHandle) {
    {
        let _guard = history_lock();
        save_history(app, &[]);
    }
    let _ = app.emit_to("settings", "history-updated", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_drops_only_expired_entries() {
        let hour = 3600 * 1000;
        let now = 1_000 * hour;
        let entry = |ago: u64| HistoryEntry { text: String::new(), ts_ms: now - ago, duration_ms: 0 };
        let mut h = vec![entry(hour), entry(23 * hour), entry(25 * hour), entry(9 * 24 * hour)];
        assert!(!retain_recent(&mut h.clone(), "forever", now));
        assert!(retain_recent(&mut h, "24h", now));
        assert_eq!(h.len(), 2);
        assert!(!retain_recent(&mut h, "24h", now));
    }

    #[test]
    fn atomic_writes_use_distinct_temp_files() {
        let dir = std::env::temp_dir().join(format!("voxy-store-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");
        std::thread::scope(|s| {
            for i in 0..8 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..50 {
                        write_atomic(path, &format!("[{i}]")).unwrap();
                    }
                });
            }
        });
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with('[') && text.ends_with(']'), "{text}");
        // временных файлов не осталось
        let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [std::ffi::OsString::from("history.json")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
