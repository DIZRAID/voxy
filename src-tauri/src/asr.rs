//! Распознавание речи: Parakeet TDT 0.6B v3 (int8 ONNX) через sherpa-onnx.
//! Модель скачивается в app_data_dir/models и загружается в память один раз.

use anyhow::{anyhow, Context, Result};
use serde_json::json;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub const MODEL_DIR_NAME: &str = "parakeet-tdt-0.6b-v3-int8";
const HF_BASE: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main";

/// (имя файла, примерный размер в байтах — для прогресса)
const MODEL_FILES: [(&str, u64); 4] = [
    ("tokens.txt", 94_000),
    ("decoder.int8.onnx", 12_400_000),
    ("joiner.int8.onnx", 6_400_000),
    ("encoder.int8.onnx", 652_000_000),
];

/// Статус модели, разделяемый между потоками (лёгкое чтение из контроллера хоткея).
pub const STATUS_MISSING: u8 = 0;
pub const STATUS_DOWNLOADING: u8 = 1;
pub const STATUS_LOADING: u8 = 2;
pub const STATUS_READY: u8 = 3;
pub const STATUS_ERROR: u8 = 4;

pub type ModelStatus = Arc<AtomicU8>;

pub fn status_name(status: u8) -> &'static str {
    match status {
        STATUS_DOWNLOADING => "downloading",
        STATUS_LOADING => "loading",
        STATUS_READY => "ready",
        STATUS_ERROR => "error",
        _ => "missing",
    }
}

pub fn emit_status(app: &AppHandle, status: &ModelStatus, code: u8, extra: serde_json::Value) {
    status.store(code, Ordering::Relaxed);
    let mut payload = json!({ "status": status_name(code) });
    if let (Some(obj), Some(add)) = (payload.as_object_mut(), extra.as_object()) {
        for (k, v) in add {
            obj.insert(k.clone(), v.clone());
        }
    }
    let _ = app.emit_to("settings", "model-status", payload);
}

pub struct ModelPaths {
    pub dir: PathBuf,
}

impl ModelPaths {
    pub fn encoder(&self) -> PathBuf {
        self.dir.join("encoder.int8.onnx")
    }
    pub fn decoder(&self) -> PathBuf {
        self.dir.join("decoder.int8.onnx")
    }
    pub fn joiner(&self) -> PathBuf {
        self.dir.join("joiner.int8.onnx")
    }
    pub fn tokens(&self) -> PathBuf {
        self.dir.join("tokens.txt")
    }
    pub fn exists(&self) -> bool {
        MODEL_FILES
            .iter()
            .all(|(name, _)| self.dir.join(name).is_file())
    }
}

pub fn model_paths(app: &AppHandle) -> ModelPaths {
    let dir = crate::store::data_dir(app)
        .join("models")
        .join(MODEL_DIR_NAME);
    ModelPaths { dir }
}

/// Скачивает недостающие файлы модели в отдельном потоке.
/// По завершении просит worker загрузить модель (WorkerMsg::LoadModel).
pub fn spawn_download(
    app: AppHandle,
    status: ModelStatus,
    worker_tx: std::sync::mpsc::Sender<crate::WorkerMsg>,
) {
    if status.load(Ordering::Relaxed) == STATUS_DOWNLOADING {
        return;
    }
    emit_status(&app, &status, STATUS_DOWNLOADING, json!({ "progress": 0 }));

    std::thread::spawn(move || {
        let paths = model_paths(&app);
        if let Err(e) = std::fs::create_dir_all(&paths.dir) {
            emit_status(
                &app,
                &status,
                STATUS_ERROR,
                json!({ "message": format!("не удалось создать папку модели: {e}") }),
            );
            return;
        }

        let total: u64 = MODEL_FILES.iter().map(|(_, size)| size).sum();
        let mut done: u64 = 0;

        for (name, approx_size) in MODEL_FILES {
            let target = paths.dir.join(name);
            if target.is_file() {
                done += approx_size;
                continue;
            }
            match download_file(&app, &status, name, &target, done, total) {
                Ok(()) => done += approx_size,
                Err(e) => {
                    let _ = std::fs::remove_file(&target);
                    emit_status(
                        &app,
                        &status,
                        STATUS_ERROR,
                        json!({ "message": format!("ошибка скачивания {name}: {e}") }),
                    );
                    return;
                }
            }
        }

        let _ = worker_tx.send(crate::WorkerMsg::LoadModel);
    });
}

fn download_file(
    app: &AppHandle,
    status: &ModelStatus,
    name: &str,
    target: &PathBuf,
    done_before: u64,
    total: u64,
) -> Result<()> {
    let url = format!("{HF_BASE}/{name}");
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(3600))
        .call()
        .with_context(|| format!("запрос {url}"))?;

    let tmp = target.with_extension("part");
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut written: u64 = 0;
    let mut last_pct: i64 = -1;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        written += n as u64;
        let pct = ((done_before + written) * 100 / total) as i64;
        if pct != last_pct {
            last_pct = pct;
            emit_status(
                app,
                status,
                STATUS_DOWNLOADING,
                json!({ "progress": pct, "file": name }),
            );
        }
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// Обёртка над OfflineRecognizer. Живёт в потоке worker'а и не покидает его.
pub struct Transcriber {
    recognizer: OfflineRecognizer,
}

impl Transcriber {
    pub fn load(paths: &ModelPaths) -> Result<Self> {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(paths.encoder().to_string_lossy().into_owned()),
            decoder: Some(paths.decoder().to_string_lossy().into_owned()),
            joiner: Some(paths.joiner().to_string_lossy().into_owned()),
        };
        config.model_config.tokens = Some(paths.tokens().to_string_lossy().into_owned());
        config.model_config.model_type = Some("nemo_transducer".into());
        let threads = crate::platform::inference_threads();
        config.model_config.num_threads = threads;
        config.decoding_method = Some("greedy_search".into());

        let t0 = std::time::Instant::now();
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("sherpa-onnx не смог создать распознаватель"))?;
        log::info!("модель загружена за {:?}, потоков: {threads}", t0.elapsed());
        Ok(Self { recognizer })
    }

    /// Прогрев: первая инференция инициализирует сессии ONNX Runtime,
    /// чтобы первая реальная диктовка не была медленной.
    pub fn warmup(&self) {
        let silence = vec![0.0f32; 8000];
        let _ = self.transcribe(&silence, 16000);
    }

    pub fn transcribe(&self, samples: &[f32], sample_rate: u32) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, samples);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default()
    }

    /// Модель обучена на фразах до ~30 c, поэтому длинные записи режем на
    /// куски ≤25 c, выбирая границу в самой тихой точке (чтобы не резать
    /// слово посередине), и склеиваем распознанные части.
    pub fn transcribe_long(&self, samples: &[f32], sample_rate: u32) -> String {
        let rate = sample_rate as usize;
        let chunk = rate * 25;
        // небольшой хвост сверх лимита не режем — модель справится
        if samples.len() <= chunk + rate * 5 {
            return self.transcribe(samples, sample_rate);
        }

        let mut parts: Vec<String> = Vec::new();
        let mut start = 0usize;
        while start < samples.len() {
            let mut end = (start + chunk).min(samples.len());
            if end < samples.len() {
                end = quietest_point(samples, end, rate * 5 / 2);
            }
            let text = self.transcribe(&samples[start..end], sample_rate);
            if !text.is_empty() {
                parts.push(text);
            }
            start = end;
        }
        parts.join(" ")
    }
}

/// Ищет центр самого тихого ~окна в пределах ±radius от `around`,
/// чтобы резать речь по паузе, а не посреди слова.
pub fn quietest_point(samples: &[f32], around: usize, radius: usize) -> usize {
    let lo = around.saturating_sub(radius);
    let hi = (around + radius).min(samples.len());
    let win = (radius / 25).max(64); // ~100 мс при radius 2.5 c
    if hi <= lo + win {
        return around.min(samples.len());
    }

    let mut best_start = around;
    let mut best_energy = f32::MAX;
    let step = (radius / 100).max(16); // ~25 мс
    let mut pos = lo;
    while pos + win <= hi {
        let energy: f32 = samples[pos..pos + win].iter().map(|s| s * s).sum();
        if energy < best_energy {
            best_energy = energy;
            best_start = pos;
        }
        pos += step;
    }
    best_start + win / 2
}
