//! Каталог локальных моделей распознавания и их установка на диск.
//!
//! Каталог встроен в приложение (catalog.json): метки качества, языки и
//! закреплённые ревизии файлов с SHA-256. Источники моделей не публикуют
//! ни качество, ни языки, поэтому эти данные курируются вместе с
//! релизами приложения (новинки ищет discovery.rs).
//!
//! Установка: models/<dir>/<файлы> + маркер .installed.json. Скачивание
//! докачивается после обрыва (.part + Range), каждый файл сверяется с
//! SHA-256 до переименования, архивы распаковываются только по белому
//! списку ожидаемых файлов.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const DEFAULT_MODEL: &str = "parakeet-tdt-0.6b-v3";
const MARKER: &str = ".installed.json";

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Catalog {
    pub version: u32,
    /// Дата сборки каталога (RFC 3339): новинки — всё, что вышло позже.
    pub generated_at: String,
    pub models: Vec<ModelSpec>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ModelSpec {
    pub id: String,
    /// Папка внутри models/ (у Parakeet v3 совпадает с прежней).
    pub dir: String,
    pub name: String,
    pub vendor: String,
    /// Семейство sherpa-onnx: nemo_transducer | whisper | qwen3_asr | moonshine.
    pub family: String,
    pub labels: Vec<String>,
    pub languages: u32,
    pub languages_note: String,
    pub english_only: bool,
    /// Оценка памяти в загруженном виде.
    pub ram_mb: u32,
    /// Размер на диске после установки.
    pub size_mb: u32,
    pub license: String,
    pub homepage: String,
    /// Имя архива модели в релизе sherpa-onnx (без .tar.bz2) — чтобы
    /// discovery не выдавал уже известные модели за новинки.
    pub upstream: String,
    /// Скорость на процессоре по замерам model_smoke: fast | medium | slow
    /// (slow — Whisper turbo, RTF ~0.35–0.67 на M-серии).
    pub speed: String,
    /// Самый длинный кусок аудио, который модель переваривает за раз.
    /// Длиннее — режется по паузам (asr::split_long). У экспорта
    /// Moonshine v2 это ~9 c: на 10 c ONNX Runtime падает на broadcast.
    #[serde(default = "default_max_chunk_s")]
    pub max_chunk_s: f32,
    pub source: Source,
    pub files: Vec<FileSpec>,
}

fn default_max_chunk_s() -> f32 {
    28.0
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Отдельные файлы: base_url + "/" + path.
    Files { base_url: String },
    /// Один архив .tar.bz2 с файлами внутри (в любой подпапке).
    Archive { url: String, size: u64, sha256: String },
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct FileSpec {
    pub role: String,
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl ModelSpec {
    /// Абсолютный путь файла с данной ролью (первый подходящий).
    pub fn file(&self, dir: &Path, role: &str) -> Result<String> {
        self.files
            .iter()
            .find(|f| f.role == role)
            .map(|f| dir.join(&f.path).to_string_lossy().into_owned())
            .ok_or_else(|| anyhow!("в каталоге у {} нет файла роли {role}", self.id))
    }

    pub fn download_bytes(&self) -> u64 {
        match &self.source {
            Source::Archive { size, .. } => *size,
            Source::Files { .. } => self.files.iter().map(|f| f.size).sum(),
        }
    }
}

pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("catalog.json")).expect("встроенный catalog.json повреждён")
    })
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    catalog().models.iter().find(|m| m.id == id)
}

pub fn models_root(app: &AppHandle) -> PathBuf {
    crate::store::data_dir(app).join("models")
}

pub fn model_dir(app: &AppHandle, spec: &ModelSpec) -> PathBuf {
    models_root(app).join(&spec.dir)
}

/// Все файлы на месте и нужного размера. Маркер не обязателен: Parakeet v3,
/// скачанная старыми версиями приложения, маркера не имеет.
pub fn is_installed(app: &AppHandle, spec: &ModelSpec) -> bool {
    files_present(&model_dir(app, spec), spec)
}

pub fn files_present(dir: &Path, spec: &ModelSpec) -> bool {
    spec.files.iter().all(|f| match fs::metadata(dir.join(&f.path)) {
        Ok(m) => m.is_file() && (f.size == 0 || m.len() == f.size),
        Err(_) => false,
    })
}

pub fn delete(app: &AppHandle, spec: &ModelSpec) -> Result<()> {
    let dir = model_dir(app, spec);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("удаление {}", dir.display()))?;
    }
    let _ = fs::remove_dir_all(staging_dir(app, spec));
    Ok(())
}

fn staging_dir(app: &AppHandle, spec: &ModelSpec) -> PathBuf {
    models_root(app).join(".downloads").join(&spec.id)
}

// ------------------------------------------------------------- скачивание

/// Активные скачивания: id → флаг отмены. Одна модель качается один раз.
#[derive(Default)]
pub struct Downloads {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Downloads {
    pub fn is_active(&self, id: &str) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    pub fn cancel(&self, id: &str) {
        if let Some(flag) = self.active.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

pub fn downloads() -> &'static Downloads {
    static D: OnceLock<Downloads> = OnceLock::new();
    D.get_or_init(Downloads::default)
}

fn emit_progress(app: &AppHandle, id: &str, state: &str, done: u64, total: u64, msg: Option<&str>) {
    let progress = if total > 0 { (done * 100 / total).min(100) } else { 0 };
    let _ = app.emit_to(
        "settings",
        "model-download",
        json!({
            "id": id, "state": state, "progress": progress,
            "done_bytes": done, "total_bytes": total, "message": msg,
        }),
    );
}

/// Скачивает модель в фоне. `on_done(true)` — установлена и проверена.
pub fn start_download(
    app: AppHandle,
    spec: &'static ModelSpec,
    on_done: impl FnOnce(bool) + Send + 'static,
) {
    let cancel = {
        let mut active = downloads().active.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains_key(&spec.id) {
            return;
        }
        let flag = Arc::new(AtomicBool::new(false));
        active.insert(spec.id.clone(), flag.clone());
        flag
    };

    std::thread::spawn(move || {
        let result = install(&app, spec, &cancel);
        downloads()
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&spec.id);
        let total = spec.download_bytes();
        match result {
            Ok(()) => {
                log::info!("модель {} установлена", spec.id);
                emit_progress(&app, &spec.id, "done", total, total, None);
                on_done(true);
            }
            Err(e) if cancel.load(Ordering::Relaxed) => {
                log::info!("скачивание {} отменено ({e:#})", spec.id);
                emit_progress(&app, &spec.id, "cancelled", 0, total, None);
                on_done(false);
            }
            Err(e) => {
                log::error!("скачивание {}: {e:#}", spec.id);
                emit_progress(&app, &spec.id, "error", 0, total, Some(&format!("{e:#}")));
                on_done(false);
            }
        }
    });
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        // таймаут на одно чтение из сокета, а не на всё скачивание:
        // гигабайтная модель на медленном канале качается долго
        .timeout_read(Duration::from_secs(60))
        // Hugging Face и GitHub перенаправляют на свои CDN только по https.
        .https_only(true)
        .build()
}

fn install(app: &AppHandle, spec: &'static ModelSpec, cancel: &AtomicBool) -> Result<()> {
    install_into(spec, &model_dir(app, spec), &staging_dir(app, spec), cancel, |state, done, total| {
        emit_progress(app, &spec.id, state, done, total, None)
    })
}

/// Установка модели в произвольную папку без Tauri (её же вызывает
/// тестовая утилита examples/model_smoke.rs). `progress(state, done, total)`.
pub fn install_into(
    spec: &ModelSpec,
    dir: &Path,
    staging: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&str, u64, u64),
) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::create_dir_all(staging)?;
    let total = spec.download_bytes();

    match &spec.source {
        Source::Files { base_url } => {
            let mut done_before = 0u64;
            for f in &spec.files {
                let target = dir.join(&f.path);
                let already = fs::metadata(&target).map(|m| m.len() == f.size).unwrap_or(false);
                if !already {
                    let part = staging.join(f.path.replace('/', "__") + ".part");
                    let url = format!("{base_url}/{}", f.path);
                    fetch(&url, &part, f.size, cancel, |n| {
                        progress("downloading", done_before + n, total)
                    })
                    .with_context(|| format!("файл {}", f.path))?;
                    progress("verifying", done_before + f.size, total);
                    if let Some(expected) = &f.sha256 {
                        verify_sha256(&part, expected).with_context(|| format!("файл {}", f.path))?;
                    }
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::rename(&part, &target)?;
                }
                done_before += f.size;
            }
        }
        Source::Archive { url, size, sha256 } => {
            let part = staging.join("archive.tar.bz2.part");
            fetch(url, &part, *size, cancel, |n| progress("downloading", n, total))?;
            progress("verifying", total, total);
            verify_sha256(&part, sha256)?;
            progress("extracting", total, total);
            extract_whitelisted(&part, dir, spec)?;
        }
    }

    if !files_present(dir, spec) {
        bail!("после установки не хватает файлов модели");
    }
    let marker = json!({ "id": spec.id, "catalog_version": catalog().version });
    fs::write(dir.join(MARKER), marker.to_string())?;
    let _ = fs::remove_dir_all(staging);
    Ok(())
}

/// Скачивание с докачкой: если .part уже частично есть — Range-запрос.
/// Сервер, не поддержавший Range (200 вместо 206), качает заново.
/// Больше `expected` байт не пишется: сервер (или CDN), присылающий лишнее,
/// не заполнит диск до проверки SHA-256.
fn fetch(
    url: &str,
    part: &Path,
    expected: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if expected > 0 && have == expected {
        return Ok(());
    }
    let have = if expected > 0 && have > expected { 0 } else { have };

    // identity: без сжатия, чтобы Content-Length был размером самого файла
    // (сжатый ответ ureq распаковал бы сам).
    let mut req = agent().get(url).set("Accept-Encoding", "identity");
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = match req.call() {
        // Докачивать нечего или нечем: .part не подходит к файлу на сервере.
        // Удаляем, чтобы следующая попытка начала сначала.
        Err(ureq::Error::Status(416, _)) => {
            let _ = fs::remove_file(part);
            bail!("сервер отверг докачку (HTTP 416), скачайте заново");
        }
        r => r.with_context(|| format!("запрос {url}"))?,
    };
    let resumed = have > 0 && resp.status() == 206;
    if expected > 0 {
        let remaining = if resumed { expected - have } else { expected };
        let encoded = resp
            .header("Content-Encoding")
            .is_some_and(|e| !e.eq_ignore_ascii_case("identity"));
        if let Some(len) = resp.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok()) {
            if !encoded && len != remaining {
                bail!("сервер отдаёт {len} байт вместо {remaining}");
            }
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)?;
    let mut written = if resumed { have } else { 0 };
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("отменено");
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if expected > 0 && written + n as u64 > expected {
            drop(file);
            let _ = fs::remove_file(part);
            bail!("сервер прислал больше {expected} байт");
        }
        file.write_all(&buf[..n])?;
        written += n as u64;
        if last_emit.elapsed() >= Duration::from_millis(250) {
            last_emit = Instant::now();
            progress(written);
        }
    }
    file.flush()?;
    progress(written);
    if expected > 0 && written != expected {
        bail!("получено {written} байт вместо {expected}");
    }
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(path);
        bail!("контрольная сумма не совпала (файл удалён, скачайте заново)");
    }
    Ok(())
}

/// Распаковывает из архива ТОЛЬКО файлы из каталога модели (по имени,
/// в какой бы подпапке архива они ни лежали). Пути из архива никогда не
/// используются для записи — обход каталога через «..» невозможен.
fn extract_whitelisted(archive: &Path, dir: &Path, spec: &ModelSpec) -> Result<()> {
    let wanted: HashMap<&str, &FileSpec> = spec
        .files
        .iter()
        .map(|f| (f.path.rsplit('/').next().unwrap_or(&f.path), f))
        .collect();
    let reader = bzip2::read::BzDecoder::new(fs::File::open(archive)?);
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if let Some(spec_file) = wanted.get(name) {
            let target = dir.join(&spec_file.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Встроенный каталог разбирается, у каждой модели есть все файлы,
    /// которые требует её семейство, и сумма SHA-256 у каждого файла.
    #[test]
    fn catalog_is_consistent() {
        let c = catalog();
        assert!(find(DEFAULT_MODEL).is_some());
        let mut ids = std::collections::HashSet::new();
        for m in &c.models {
            assert!(ids.insert(&m.id), "дубль id {}", m.id);
            let roles: &[&str] = match m.family.as_str() {
                "nemo_transducer" => &["encoder", "decoder", "joiner", "tokens"],
                "whisper" => &["encoder", "decoder", "tokens"],
                "qwen3_asr" => &["conv_frontend", "encoder", "decoder", "tokenizer"],
                "moonshine" => &["encoder", "merged_decoder", "tokens"],
                other => panic!("{}: неизвестное семейство {other}", m.id),
            };
            for r in roles {
                assert!(m.files.iter().any(|f| f.role == *r), "{}: нет роли {r}", m.id);
            }
            match &m.source {
                Source::Files { base_url } => {
                    assert!(base_url.starts_with("https://huggingface.co/"), "{}", m.id);
                    assert!(!base_url.contains("/resolve/main"), "{}: ревизия не закреплена", m.id);
                    for f in &m.files {
                        assert!(f.size > 0 && f.sha256.as_deref().is_some_and(|s| s.len() == 64),
                            "{}: {} без размера или SHA-256", m.id, f.path);
                    }
                }
                Source::Archive { url, sha256, size } => {
                    assert!(url.starts_with("https://github.com/k2-fsa/"), "{}", m.id);
                    assert!(sha256.len() == 64 && *size > 0, "{}", m.id);
                }
            }
            assert!(["fast", "medium", "slow"].contains(&m.speed.as_str()), "{}", m.id);
        }
    }
}
