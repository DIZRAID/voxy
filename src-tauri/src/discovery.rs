//! Поиск новых моделей без собственного сервера: раз в сутки приложение
//! одним запросом читает официальный релиз моделей sherpa-onnx на GitHub
//! (все ~500 моделей приходят в одном ответе; лимит GitHub для запросов
//! без ключа — 60 в час) и показывает то, что вышло после сборки
//! встроенного каталога и относится к семействам, которые Typely умеет
//! запускать.
//!
//! Качество и языки источник не сообщает, поэтому новинки показываются
//! как «ещё без оценки» и не ставятся автоматически: у каждой модели свои
//! особенности (у Moonshine v2, например, предел длины куска ~9 c), и
//! без проверки она могла бы молча ломать распознавание. Проверенные
//! модели попадают в каталог с обновлением приложения.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::models;

const RELEASE_API: &str =
    "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/asr-models";
pub const RELEASE_PAGE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models";
const CHECK_EVERY_MS: u64 = 24 * 3600 * 1000;
const MAX_ITEMS: usize = 5;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Found {
    pub title: String,
    pub family: String,
    pub size_mb: u64,
    pub released: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    checked_at_ms: u64,
    items: Vec<Found>,
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    updated_at: String,
}

fn cache_path(app: &AppHandle) -> std::path::PathBuf {
    crate::store::data_dir(app).join("discovery.json")
}

fn load_cache(app: &AppHandle) -> Cache {
    std::fs::read_to_string(cache_path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Последний найденный список (без сети).
pub fn cached(app: &AppHandle) -> Vec<Found> {
    load_cache(app).items
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Проверяет релиз в фоне, если с прошлой проверки прошло больше суток.
pub fn spawn_check(app: AppHandle) {
    if now_ms().saturating_sub(load_cache(&app).checked_at_ms) < CHECK_EVERY_MS {
        return;
    }
    std::thread::spawn(move || match fetch_new_models() {
        Ok(items) => {
            log::info!("discovery: новых моделей — {}", items.len());
            let cache = Cache {
                checked_at_ms: now_ms(),
                items,
            };
            if let Ok(json) = serde_json::to_string_pretty(&cache) {
                let _ = std::fs::write(cache_path(&app), json);
            }
            let _ = app.emit_to("settings", "models-changed", ());
        }
        // Нет сети — не страшно: попробуем при следующем запуске.
        Err(e) => log::warn!("discovery: {e:#}"),
    });
}

fn fetch_new_models() -> Result<Vec<Found>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();
    let body = agent
        .get(RELEASE_API)
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("запрос к GitHub")?
        .into_string()?;
    let release: Release = serde_json::from_str(&body).context("разбор ответа GitHub")?;

    let catalog = models::catalog();
    let mut found: Vec<Found> = release
        .assets
        .into_iter()
        .filter(|a| a.updated_at > catalog.generated_at)
        .filter_map(|a| {
            let stem = a.name.strip_suffix(".tar.bz2")?;
            if catalog.models.iter().any(|m| m.upstream == stem) {
                return None;
            }
            let family = family_of(stem)?;
            Some(Found {
                title: pretty_title(stem),
                family: family.to_string(),
                size_mb: a.size / 1_000_000,
                released: a.updated_at.chars().take(10).collect(),
            })
        })
        .collect();
    found.sort_by(|a, b| b.released.cmp(&a.released));
    found.truncate(MAX_ITEMS);
    Ok(found)
}

/// Семейство, которое Typely умеет запускать, или None. Сборки под NPU
/// (Rockchip, Ascend, Qualcomm…), стриминговые и CTC-варианты — мимо.
fn family_of(stem: &str) -> Option<&'static str> {
    let n = stem.to_ascii_lowercase();
    const SKIP: [&str; 10] = [
        "rk35", "rknn", "ascend", "qnn", "axera", "ax650", "horizon", "streaming", "ctc", "online",
    ];
    if SKIP.iter().any(|s| n.contains(s)) {
        return None;
    }
    if n.contains("parakeet") && n.contains("tdt") {
        Some("nemo_transducer")
    } else if n.contains("whisper") {
        Some("whisper")
    } else if n.contains("moonshine") {
        Some("moonshine")
    } else if n.contains("qwen3-asr") {
        Some("qwen3_asr")
    } else {
        None
    }
}

fn pretty_title(stem: &str) -> String {
    stem.trim_start_matches("sherpa-onnx-")
        .trim_start_matches("nemo-")
        .replace(['-', '_'], " ")
}

#[cfg(test)]
mod tests {
    use super::family_of;

    #[test]
    fn family_filter() {
        assert_eq!(family_of("sherpa-onnx-nemo-parakeet-tdt-0.6b-v4-int8"), Some("nemo_transducer"));
        assert_eq!(family_of("sherpa-onnx-whisper-large-v4"), Some("whisper"));
        assert_eq!(family_of("sherpa-onnx-qwen3-asr-1.7B-int8-2026-10-01"), Some("qwen3_asr"));
        assert_eq!(family_of("sherpa-onnx-moonshine-base-en-quantized-2026-11-01"), Some("moonshine"));
        // NPU-сборки, стриминг, CTC и чужие семейства — мимо
        assert_eq!(family_of("sherpa-onnx-rk3588-parakeet-tdt-0.6b-v3"), None);
        assert_eq!(family_of("sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8"), None);
        assert_eq!(family_of("sherpa-onnx-nemotron-3.5-asr-streaming-0.6b"), None);
        assert_eq!(family_of("sherpa-onnx-zipformer-en-2023-06-26"), None);
    }
}
