//! Сквозная проверка модели из каталога без UI и микрофона:
//!   cargo run --release --example model_smoke -- <model-id> <файл.wav> <папка-моделей>
//!
//! Скачивает модель тем же кодом, что и приложение (докачка, SHA-256,
//! распаковка архива), загружает её и распознаёт WAV. Печатает время.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use voice_lib::{asr::Transcriber, models};

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "model_smoke <model-id> <файл.wav> <папка-моделей>";
    let id = args.next().expect(usage);
    let wav_path = args.next().expect(usage);
    let root = PathBuf::from(args.next().expect(usage));

    let spec = models::find(&id).unwrap_or_else(|| {
        let ids: Vec<_> = models::catalog().models.iter().map(|m| m.id.as_str()).collect();
        panic!("нет модели {id}; есть: {ids:?}")
    });
    let dir = root.join(&spec.dir);
    let staging = root.join(".downloads").join(&spec.id);

    let t0 = Instant::now();
    let mut last = String::new();
    models::install_into(spec, &dir, &staging, &AtomicBool::new(false), |state, done, total| {
        let line = format!("{state} {}%", if total > 0 { done * 100 / total } else { 0 });
        if line != last && (state != "downloading" || done == total || done % (50 << 20) < (1 << 20)) {
            println!("  {line}");
            last = line;
        }
    })
    .expect("установка не удалась");
    println!("установлена за {:?} ({} МБ на диске)", t0.elapsed(), spec.size_mb);

    let (samples, rate) = read_wav(&wav_path);
    println!("аудио: {:.2} c, {rate} Гц", samples.len() as f32 / rate as f32);

    let t1 = Instant::now();
    let t = Transcriber::load(spec, &dir).expect("модель не загрузилась");
    println!("загрузка {:?}", t1.elapsed());
    let t2 = Instant::now();
    t.warmup();
    println!("прогрев {:?}", t2.elapsed());
    let t3 = Instant::now();
    let text = t.transcribe_long(&samples, rate);
    let dt = t3.elapsed();
    println!(
        "распознано за {dt:?} (RTF {:.3})",
        dt.as_secs_f32() / (samples.len() as f32 / rate as f32)
    );
    println!("ТЕКСТ: {text}");
}

fn read_wav(path: &str) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path).expect("не открыть WAV");
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };
    let mono = if channels > 1 {
        raw.chunks_exact(channels)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        raw
    };
    (mono, spec.sample_rate)
}
