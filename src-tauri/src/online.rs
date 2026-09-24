//! Онлайн-распознавание через API с ключом пользователя.
//!
//! OpenAI и Groq говорят на одном OpenAI-совместимом протоколе
//! (/v1/audio/transcriptions), ElevenLabs — на своём. Ключи хранятся
//! только в системном хранилище (Связка ключей macOS / Windows Credential
//! Manager) и никогда не уходят во фронтенд целиком.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const KEYCHAIN_SERVICE: &str = "com.dizraid.voice";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    OpenAiCompatible,
    ElevenLabs,
}

#[derive(Serialize, Clone, Copy)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub model: &'static str,
    /// Цена за минуту аудио в USD (сентябрь 2026) — для подсказки в UI.
    pub usd_per_min: f64,
    pub languages_note: &'static str,
    /// Где пользователь берёт ключ.
    pub key_url: &'static str,
    #[serde(skip)]
    pub protocol: Protocol,
    #[serde(skip)]
    pub base_url: &'static str,
}

pub const PROVIDERS: [Provider; 3] = [
    Provider {
        id: "openai",
        name: "OpenAI",
        model: "gpt-transcribe",
        usd_per_min: 0.0045,
        languages_note: "Dozens of languages",
        key_url: "https://platform.openai.com/api-keys",
        protocol: Protocol::OpenAiCompatible,
        base_url: "https://api.openai.com/v1",
    },
    Provider {
        id: "groq",
        name: "Groq",
        model: "whisper-large-v3-turbo",
        usd_per_min: 0.00067,
        languages_note: "99 languages",
        key_url: "https://console.groq.com/keys",
        protocol: Protocol::OpenAiCompatible,
        base_url: "https://api.groq.com/openai/v1",
    },
    Provider {
        id: "elevenlabs",
        name: "ElevenLabs",
        model: "scribe_v2",
        usd_per_min: 0.0037,
        languages_note: "90+ languages",
        key_url: "https://elevenlabs.io/app/settings/api-keys",
        protocol: Protocol::ElevenLabs,
        base_url: "https://api.elevenlabs.io/v1",
    },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

// ------------------------------------------------------------------ ключи

fn entry(provider_id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("api-key:{provider_id}"))
        .map_err(|e| anyhow!("хранилище ключей недоступно: {e}"))
}

pub fn get_key(provider_id: &str) -> Option<String> {
    entry(provider_id).ok()?.get_password().ok().filter(|k| !k.is_empty())
}

/// Есть ли ключ — с кешем: каждое чтение из Связки ключей у приложения
/// без постоянной подписи может спросить разрешение у пользователя.
pub fn has_key(provider_id: &str) -> bool {
    let mut cache = key_cache().lock().unwrap_or_else(|e| e.into_inner());
    *cache
        .entry(provider_id.to_string())
        .or_insert_with(|| get_key(provider_id).is_some())
}

fn key_cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set_key(provider_id: &str, key: &str) -> Result<()> {
    entry(provider_id)?
        .set_password(key)
        .map_err(|e| anyhow!("Could not save the key: {e}"))?;
    key_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider_id.to_string(), true);
    Ok(())
}

pub fn delete_key(provider_id: &str) -> Result<()> {
    match entry(provider_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(anyhow!("Could not remove the key: {e}")),
    }
    key_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider_id.to_string(), false);
    Ok(())
}

/// Ключ из поля ввода: без пробелов по краям и только печатный ASCII.
/// Невидимые символы (U+200B, U+FEFF), неразрывный пробел внутри ключа и
/// перенос строки, ставший пробелом, отвергаются до любого запроса: ни у
/// одного провайдера таких символов в ключе нет, а ureq вернул бы ошибку
/// BadHeader, текст которой содержит весь заголовок вместе с ключом.
pub fn clean_key(raw: &str) -> Result<&str, String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err("Paste an API key".into());
    }
    if key.len() > 1024 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(
            "The key contains spaces or invisible characters. Copy it again from the provider's page"
                .into(),
        );
    }
    Ok(key)
}

/// Ошибка запроса для лога — без текста самой ошибки: у BadHeader он
/// содержит заголовок целиком (Authorization / xi-api-key с ключом).
/// Причина (сеть, TLS) берётся из source(), в котором заголовков нет.
fn describe(e: &ureq::Error) -> String {
    use std::error::Error as _;
    match e.source() {
        Some(cause) => format!("{}: {cause}", e.kind()),
        None => e.kind().to_string(),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        // Никаких перенаправлений на http://.
        .https_only(true)
        .build()
}

/// Проверяет ключ дешёвым запросом, не расходуя минуты распознавания.
pub fn verify_key(p: &Provider, key: &str) -> Result<()> {
    let req = match p.protocol {
        Protocol::OpenAiCompatible => agent()
            .get(&format!("{}/models", p.base_url))
            .set("Authorization", &format!("Bearer {key}")),
        Protocol::ElevenLabs => agent()
            .get(&format!("{}/user", p.base_url))
            .set("xi-api-key", key),
    };
    match req.call() {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(401 | 403, _)) => bail!("{} rejected this API key", p.name),
        Err(ureq::Error::Status(code, _)) => bail!("{}: server error {code}", p.name),
        Err(e) => {
            log::warn!("проверка ключа {}: {}", p.name, describe(&e));
            bail!("No connection to {}", p.name)
        }
    }
}

// ----------------------------------------------------------- распознавание

pub struct OnlineEngine {
    pub provider: &'static Provider,
    key: String,
}

impl OnlineEngine {
    pub fn new(provider_id: &str) -> Result<Self> {
        let provider =
            provider(provider_id).ok_or_else(|| anyhow!("Unknown provider {provider_id}"))?;
        let key = get_key(provider_id)
            .ok_or_else(|| anyhow!("No API key for {}", provider.name))?;
        Ok(Self { provider, key })
    }

    /// Распознаёт кусок записи. Длинные записи режутся вызывающим кодом
    /// (asr::split_long) — каждый запрос остаётся маленьким (~0.8 МБ WAV).
    pub fn transcribe(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        let wav = to_wav_16k(samples, sample_rate)?;
        let p = self.provider;
        let (url, auth_header, auth_value, fields) = match p.protocol {
            Protocol::OpenAiCompatible => (
                format!("{}/audio/transcriptions", p.base_url),
                "Authorization",
                format!("Bearer {}", self.key),
                vec![("model", p.model), ("response_format", "json")],
            ),
            Protocol::ElevenLabs => (
                format!("{}/speech-to-text", p.base_url),
                "xi-api-key",
                self.key.clone(),
                vec![("model_id", p.model)],
            ),
        };
        let (content_type, body) = multipart(&fields, "audio.wav", "audio/wav", &wav);

        let resp = agent()
            .post(&url)
            .set(auth_header, &auth_value)
            .set("Content-Type", &content_type)
            .send_bytes(&body);
        let text = match resp {
            Ok(r) => r.into_string().context("чтение ответа")?,
            Err(ureq::Error::Status(401 | 403, _)) => bail!("{}: invalid API key", p.name),
            Err(ureq::Error::Status(429, _)) => bail!("{}: rate limit or no credits", p.name),
            Err(ureq::Error::Status(code, r)) => {
                let detail: String = r.into_string().unwrap_or_default().chars().take(500).collect();
                log::error!("{} HTTP {code}: {detail}", p.name);
                bail!("{}: server error {code}", p.name)
            }
            Err(e) => {
                log::error!("{}: {}", p.name, describe(&e));
                bail!("No connection to {}", p.name)
            }
        };
        let json: serde_json::Value = serde_json::from_str(&text).context("разбор ответа")?;
        Ok(json["text"].as_str().unwrap_or_default().trim().to_string())
    }
}

/// Моно f32 → WAV 16 кГц 16 бит: так запрос в 6 раз меньше, чем 48 кГц f32,
/// а распознаватели всё равно работают на 16 кГц.
fn to_wav_16k(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    const OUT_RATE: u32 = 16_000;
    let resampled;
    let pcm: &[f32] = if sample_rate == OUT_RATE {
        samples
    } else {
        let r = sherpa_onnx::LinearResampler::create(sample_rate as i32, OUT_RATE as i32)
            .ok_or_else(|| anyhow!("ресемплер {sample_rate} → {OUT_RATE} не создан"))?;
        resampled = r.resample(samples, true);
        &resampled
    };

    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // размер fmt-чанка
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // моно
    out.extend_from_slice(&OUT_RATE.to_le_bytes());
    out.extend_from_slice(&(OUT_RATE * 2).to_le_bytes()); // байт в секунду
    out.extend_from_slice(&2u16.to_le_bytes()); // байт на кадр
    out.extend_from_slice(&16u16.to_le_bytes()); // бит на сэмпл
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(out)
}

fn multipart(
    fields: &[(&str, &str)],
    filename: &str,
    file_type: &str,
    file: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!(
        "voxy-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let mut body = Vec::with_capacity(file.len() + 512);
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {file_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_and_resampling() {
        let one_second_48k = vec![0.25f32; 48_000];
        let wav = to_wav_16k(&one_second_48k, 48_000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        assert_eq!(rate, 16_000);
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        // ~1 c при 16 кГц, 16 бит (ресемплер может дать ±несколько сэмплов)
        assert!((data_len as i64 / 2 - 16_000).abs() < 64, "{data_len}");
    }

    #[test]
    fn keys_with_invisible_characters_are_rejected() {
        assert_eq!(clean_key("  sk-proj-AbC_123-xyz \n"), Ok("sk-proj-AbC_123-xyz"));
        assert_eq!(clean_key(" \t "), Err("Paste an API key".into()));
        for bad in [
            "sk-proj-abc\u{200B}",  // zero-width space: trim() его не убирает
            "\u{FEFF}sk-proj-abc",  // BOM
            "sk-proj-abc def",     // перенос строки, ставший пробелом
            "sk-proj-abc\u{00A0}def",
            "sk-proj-abc\ndef",
            "sk-проект",
        ] {
            assert!(clean_key(bad).is_err(), "{bad:?}");
        }
        assert!(clean_key(&"k".repeat(1025)).is_err());
    }

    /// Ошибка заголовка у ureq содержит весь заголовок с ключом; в лог
    /// уходит только её вид. Без сети: заголовок проверяется до соединения
    /// (а 127.0.0.1:9 и так никуда не ведёт).
    #[test]
    fn logged_errors_never_contain_the_key() {
        let secret = "sk-proj-SECRETKEY";
        let err = agent()
            .get("https://127.0.0.1:9/")
            .set("xi-api-key", &format!("{secret}\u{200B}"))
            .call()
            .unwrap_err();
        assert!(err.to_string().contains(secret), "ureq больше не печатает заголовок?");
        let logged = describe(&err);
        assert!(!logged.contains(secret), "{logged}");
    }

    #[test]
    fn multipart_contains_fields_and_file() {
        let (ct, body) = multipart(&[("model", "m1")], "audio.wav", "audio/wav", b"DATA");
        let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"model\"\r\n\r\nm1\r\n"));
        assert!(text.contains("filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\nDATA\r\n"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    /// Сетевой: каждый провайдер отвергает заведомо ложный ключ понятной
    /// ошибкой. Запуск: cargo test -- --ignored fake_keys
    #[test]
    #[ignore]
    fn fake_keys_are_rejected_clearly() {
        for p in &PROVIDERS {
            let err = verify_key(p, "sk-voxy-fake-key-000000").unwrap_err().to_string();
            println!("{}: {err}", p.name);
            assert!(err.contains("rejected"), "{}: {err}", p.name);
            let engine = OnlineEngine { provider: p, key: "sk-voxy-fake-key-000000".into() };
            let err = engine.transcribe(&vec![0.0; 16_000], 16_000).unwrap_err().to_string();
            println!("{} transcribe: {err}", p.name);
            assert!(err.contains("invalid API key"), "{}: {err}", p.name);
        }
    }
}
