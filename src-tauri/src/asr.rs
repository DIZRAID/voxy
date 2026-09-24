//! Локальное распознавание речи через sherpa-onnx. Поддерживаемые
//! семейства моделей (см. catalog.json): NeMo-трансдьюсеры (Parakeet),
//! Whisper, Qwen3-ASR, Moonshine v2.

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use sherpa_onnx::{
    OfflineQwen3ASRModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineTransducerModelConfig, OfflineWhisperModelConfig,
};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

use crate::models::ModelSpec;

/// Статус активного движка, разделяемый между потоками (лёгкое чтение
/// из контроллера хоткея).
pub const STATUS_MISSING: u8 = 0;
pub const STATUS_DOWNLOADING: u8 = 1;
pub const STATUS_LOADING: u8 = 2;
pub const STATUS_READY: u8 = 3;
pub const STATUS_ERROR: u8 = 4;
/// Модель на диске, но выгружена из памяти после простоя: запись
/// разрешена, загрузка идёт параллельно с ней.
pub const STATUS_UNLOADED: u8 = 5;

pub type ModelStatus = Arc<AtomicU8>;

pub fn status_name(status: u8) -> &'static str {
    match status {
        STATUS_DOWNLOADING => "downloading",
        STATUS_LOADING => "loading",
        STATUS_READY => "ready",
        STATUS_ERROR => "error",
        STATUS_UNLOADED => "unloaded",
        _ => "missing",
    }
}

/// Можно ли начинать запись: модель готова или будет готова к моменту,
/// когда запись закончится.
pub fn can_record(status: u8) -> bool {
    matches!(status, STATUS_READY | STATUS_UNLOADED | STATUS_LOADING)
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

/// Обёртка над OfflineRecognizer. Живёт в потоке worker'а и не покидает его.
pub struct Transcriber {
    recognizer: OfflineRecognizer,
    pub model_id: String,
    max_chunk_s: f32,
}

impl Transcriber {
    pub fn load(spec: &ModelSpec, dir: &Path) -> Result<Self> {
        let mut config = OfflineRecognizerConfig::default();
        let mc = &mut config.model_config;
        match spec.family.as_str() {
            "nemo_transducer" => {
                mc.transducer = OfflineTransducerModelConfig {
                    encoder: Some(spec.file(dir, "encoder")?),
                    decoder: Some(spec.file(dir, "decoder")?),
                    joiner: Some(spec.file(dir, "joiner")?),
                };
                mc.tokens = Some(spec.file(dir, "tokens")?);
                mc.model_type = Some("nemo_transducer".into());
            }
            "whisper" => {
                mc.whisper = OfflineWhisperModelConfig {
                    encoder: Some(spec.file(dir, "encoder")?),
                    decoder: Some(spec.file(dir, "decoder")?),
                    // пустой язык = автоопределение
                    language: Some(String::new()),
                    task: Some("transcribe".into()),
                    ..Default::default()
                };
                mc.tokens = Some(spec.file(dir, "tokens")?);
            }
            "qwen3_asr" => {
                // tokenizer — папка, в которой лежат vocab.json и merges.txt
                let tokenizer = Path::new(&spec.file(dir, "tokenizer")?)
                    .parent()
                    .ok_or_else(|| anyhow!("нет папки tokenizer"))?
                    .to_string_lossy()
                    .into_owned();
                mc.qwen3_asr = OfflineQwen3ASRModelConfig {
                    conv_frontend: Some(spec.file(dir, "conv_frontend")?),
                    encoder: Some(spec.file(dir, "encoder")?),
                    decoder: Some(spec.file(dir, "decoder")?),
                    tokenizer: Some(tokenizer),
                    // По умолчанию 128 новых токенов: быстрой речи в куске
                    // ~26 c может не хватить. Контекст = ~330 аудио-токенов
                    // + промпт + ответ, отсюда запас 1024.
                    max_new_tokens: 256,
                    max_total_len: 1024,
                    ..Default::default()
                };
                mc.tokens = Some(String::new());
            }
            "moonshine" => {
                mc.moonshine.encoder = Some(spec.file(dir, "encoder")?);
                mc.moonshine.merged_decoder = Some(spec.file(dir, "merged_decoder")?);
                mc.tokens = Some(spec.file(dir, "tokens")?);
            }
            other => bail!("неизвестное семейство модели: {other}"),
        }
        let threads = crate::platform::inference_threads();
        mc.num_threads = threads;
        config.decoding_method = Some("greedy_search".into());

        let t0 = std::time::Instant::now();
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("sherpa-onnx не смог загрузить модель {}", spec.id))?;
        log::info!(
            "модель {} загружена за {:?}, потоков: {threads}",
            spec.id,
            t0.elapsed()
        );
        Ok(Self {
            recognizer,
            model_id: spec.id.clone(),
            max_chunk_s: spec.max_chunk_s,
        })
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

    /// Длинные записи режем на куски не длиннее предела модели (Whisper —
    /// строго ≤30 c, Moonshine v2 — ~9 c), выбирая границу в самой тихой
    /// точке, чтобы не резать слово посередине, и склеиваем части.
    pub fn transcribe_long(&self, samples: &[f32], sample_rate: u32) -> String {
        split_long(samples, sample_rate, self.max_chunk_s, |piece| {
            self.transcribe(piece, sample_rate)
        })
        .join(" ")
    }
}

/// Режет запись на куски короче `max_s` по тихим точкам и прогоняет
/// каждый через `f`. Пустые результаты отбрасываются. Общая логика для
/// локальных и онлайн-движков (онлайн: `ONLINE_MAX_CHUNK_S`).
pub fn split_long(
    samples: &[f32],
    sample_rate: u32,
    max_s: f32,
    mut f: impl FnMut(&[f32]) -> String,
) -> Vec<String> {
    let rate = sample_rate as f32;
    if samples.len() as f32 <= max_s * rate {
        let text = f(samples);
        return if text.is_empty() { vec![] } else { vec![text] };
    }

    // Цель — 6/7 предела, поиск паузы ±1/14: кусок всегда < 13/14 предела
    // (для 28 c: цель 24 c ± 2 c).
    let target = (max_s * rate * 6.0 / 7.0) as usize;
    let radius = (max_s * rate / 14.0) as usize;
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    while start < samples.len() {
        let rest = &samples[start..];
        let len = if rest.len() as f32 <= max_s * rate {
            rest.len()
        } else {
            quietest_point(rest, target, radius).max(1)
        };
        let text = f(&rest[..len]);
        if !text.is_empty() {
            parts.push(text);
        }
        start += len;
    }
    parts
}

/// Предел куска для онлайн-провайдеров: запросы остаются маленькими
/// (~0.8 МБ WAV), а распознавание — точным.
pub const ONLINE_MAX_CHUNK_S: f32 = 28.0;

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

#[cfg(test)]
mod tests {
    use super::split_long;

    /// Куски не длиннее предела, покрывают запись целиком и без пересечений.
    #[test]
    fn split_long_respects_limit_and_covers_everything() {
        let rate = 16_000u32;
        for (secs, max_s) in [(5.0f32, 28.0f32), (35.0, 28.0), (95.0, 28.0), (35.0, 8.0), (61.0, 8.0)] {
            let n = (secs * rate as f32) as usize;
            // «речь» с паузами каждые 3 c, чтобы было где резать
            let samples: Vec<f32> = (0..n)
                .map(|i| if (i / rate as usize) % 3 == 2 { 0.0 } else { ((i as f32) * 0.05).sin() * 0.3 })
                .collect();
            let mut covered = 0usize;
            let mut max_piece = 0usize;
            split_long(&samples, rate, max_s, |piece| {
                covered += piece.len();
                max_piece = max_piece.max(piece.len());
                "x".into()
            });
            assert_eq!(covered, n, "{secs} c / предел {max_s}: покрыто не всё");
            assert!(
                max_piece as f32 <= max_s * rate as f32,
                "{secs} c / предел {max_s}: кусок {max_piece} длиннее предела"
            );
        }
    }
}
