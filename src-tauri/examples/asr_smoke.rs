//! Смоук-тест распознавания без UI и микрофона:
//!   cargo run --release --example asr_smoke -- путь/к/файлу.wav [папка_модели]
//!
//! Печатает время загрузки модели, время распознавания и текст.

use std::path::PathBuf;
use std::time::Instant;
use voice_lib::asr::{ModelPaths, Transcriber};

fn main() {
    let wav_path = std::env::args()
        .nth(1)
        .expect("использование: asr_smoke <файл.wav> [папка_модели]");
    let model_dir = std::env::args().nth(2).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").expect("нет $HOME"))
            .join("Library/Application Support/com.dizraid.voice/models/parakeet-tdt-0.6b-v3-int8")
    });

    let mut reader = hound::WavReader::open(&wav_path).expect("не открыть WAV");
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / max)
                .collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };
    let samples: Vec<f32> = if channels > 1 {
        raw.chunks_exact(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        raw
    };
    println!(
        "аудио: {:.2} c, {} Гц, {} канал(ов)",
        samples.len() as f32 / spec.sample_rate as f32,
        spec.sample_rate,
        channels
    );

    let paths = ModelPaths { dir: model_dir };
    assert!(paths.exists(), "модель не найдена в {:?}", paths.dir);

    let t0 = Instant::now();
    let transcriber = Transcriber::load(&paths).expect("модель не загрузилась");
    println!("модель загружена за {:?}", t0.elapsed());

    let t1 = Instant::now();
    transcriber.warmup();
    println!("прогрев за {:?}", t1.elapsed());

    let t2 = Instant::now();
    let text = transcriber.transcribe(&samples, spec.sample_rate);
    println!("распознано за {:?}", t2.elapsed());
    println!("ТЕКСТ: {text}");
}
