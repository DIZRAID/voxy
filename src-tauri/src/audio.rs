//! Захват микрофона через cpal.
//!
//! Realtime-гигиена (урок инцидента с зависанием): аудио-колбэк CoreAudio
//! не делает НИЧЕГО, кроме записи в буфер и атомиков — ни Tauri-IPC,
//! ни каналов, ни логов. Уровень громкости на островок шлёт отдельный
//! обычный поток-«насос» с частотой ~12 Гц: чаще островок всё равно
//! не рисует (волна семплирует раз в 110 мс), а каждое событие — это
//! сериализация, вызов скрипта на главном потоке и межпроцессное сообщение.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::hotkey::Ctrl;

const LEVEL_PUMP_INTERVAL: Duration = Duration::from_millis(83); // ~12 Гц

/// Окно для уровня громкости: 80 мс хвоста записи.
const LEVEL_WINDOW_S: f32 = 0.08;

fn lock_buf(m: &Mutex<Vec<f32>>) -> MutexGuard<'_, Vec<f32>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct RecShared {
    buf: Mutex<Vec<f32>>,
    /// Текущая длина буфера (после отрезания чанков уменьшается).
    sample_count: AtomicUsize,
    /// Всего записано за сессию (не уменьшается) — для лимита длительности.
    total_count: AtomicUsize,
    /// true, пока идёт запись; останавливает поток-насос.
    active: AtomicBool,
}

pub struct Recorder {
    stream: Option<cpal::Stream>,
    shared: Arc<RecShared>,
    sample_rate: u32,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            stream: None,
            shared: Arc::new(RecShared {
                buf: Mutex::new(Vec::new()),
                sample_count: AtomicUsize::new(0),
                total_count: AtomicUsize::new(0),
                active: AtomicBool::new(false),
            }),
            sample_rate: 0,
        }
    }

    pub fn start(
        &mut self,
        app: &AppHandle,
        device_name: Option<&str>,
        max_seconds: u64,
        ctrl_tx: Sender<Ctrl>,
    ) -> Result<()> {
        if self.stream.is_some() {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .or_else(|| host.default_input_device()),
            None => host.default_input_device(),
        }
        .ok_or_else(|| anyhow!("микрофон не найден"))?;

        let supported = device.default_input_config()?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let max_samples = (sample_rate as u64 * max_seconds) as usize;

        self.sample_rate = sample_rate;
        self.shared = Arc::new(RecShared {
            buf: Mutex::new(Vec::with_capacity(sample_rate as usize * 10)),
            sample_count: AtomicUsize::new(0),
            total_count: AtomicUsize::new(0),
            active: AtomicBool::new(true),
        });

        // Аудио-колбэк: только буфер + атомики. Никакого IPC/анализа здесь.
        let shared = self.shared.clone();
        let on_chunk = move |mono: Vec<f32>| {
            let n = mono.len();
            let mut buf = lock_buf(&shared.buf);
            buf.extend_from_slice(&mono);
            shared.sample_count.store(buf.len(), Ordering::Relaxed);
            shared.total_count.fetch_add(n, Ordering::Relaxed);
        };

        let err_cb = |e| log::error!("ошибка аудиопотока: {e}");
        let config: cpal::StreamConfig = supported.config();

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| on_chunk(downmix(data, channels)),
                err_cb,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    on_chunk(downmix(&f, channels))
                },
                err_cb,
                None,
            )?,
            other => return Err(anyhow!("неподдерживаемый формат сэмплов: {other:?}")),
        };

        stream.play()?;
        self.stream = Some(stream);
        spawn_level_pump(
            app.clone(),
            self.shared.clone(),
            sample_rate,
            max_samples,
            ctrl_tx,
        );
        Ok(())
    }

    /// Останавливает запись и возвращает (остаток моно-сэмплов, частота).
    pub fn stop(&mut self) -> (Vec<f32>, u32) {
        self.shared.active.store(false, Ordering::Relaxed);
        self.stream = None; // drop закрывает поток
        let samples = std::mem::take(&mut *lock_buf(&self.shared.buf));
        (samples, self.sample_rate)
    }

    /// Отрезает готовый кусок ~target_seconds от начала буфера (граница —
    /// в самой тихой точке рядом), не останавливая запись. Стриминговое
    /// распознавание: куски уходят в работу, пока пользователь ещё говорит.
    pub fn take_chunk(&mut self, target_seconds: usize) -> Option<(Vec<f32>, u32)> {
        let rate = self.sample_rate as usize;
        if rate == 0 {
            return None;
        }
        let target = rate * target_seconds;
        let radius = rate * 5 / 2;
        let mut buf = lock_buf(&self.shared.buf);
        if buf.len() < target + radius {
            return None;
        }
        let cut = crate::asr::quietest_point(&buf, target, radius);
        let chunk: Vec<f32> = buf.drain(..cut).collect();
        self.shared.sample_count.store(buf.len(), Ordering::Relaxed);
        Some((chunk, self.sample_rate))
    }
}

/// Обычный (не realtime) поток: раз в ~83 мс считает уровень громкости
/// по хвосту записи, шлёт его на островок и следит за лимитом длительности.
/// Завершается сам при stop().
fn spawn_level_pump(
    app: AppHandle,
    shared: Arc<RecShared>,
    sample_rate: u32,
    max_samples: usize,
    ctrl_tx: Sender<Ctrl>,
) {
    std::thread::spawn(move || {
        let window = (sample_rate as f32 * LEVEL_WINDOW_S) as usize;
        // Порог отрезания готового куска для стримингового распознавания.
        let chunk_trigger = sample_rate as usize * CHUNK_TRIGGER_S;
        let mut limit_sent = false;
        let mut last_chunk_req = std::time::Instant::now();

        while shared.active.load(Ordering::Relaxed) {
            let rms = {
                let buf = lock_buf(&shared.buf);
                let tail = &buf[buf.len().saturating_sub(window)..];
                if tail.is_empty() {
                    0.0
                } else {
                    (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
                }
            };
            let _ = app.emit_to("island", "level", perceptual_level(rms));

            if !limit_sent && shared.total_count.load(Ordering::Relaxed) >= max_samples {
                limit_sent = true;
                let _ = ctrl_tx.send(Ctrl::AutoStop);
            }

            if shared.sample_count.load(Ordering::Relaxed) >= chunk_trigger
                && last_chunk_req.elapsed().as_secs() >= 2
            {
                last_chunk_req = std::time::Instant::now();
                let _ = ctrl_tx.send(Ctrl::AutoChunk);
            }
            std::thread::sleep(LEVEL_PUMP_INTERVAL);
        }
    });
}

/// Кусок отрезается при накоплении этого объёма (цель — CHUNK_TARGET_S,
/// запас нужен, чтобы хвост записи сохранял контекст).
pub const CHUNK_TARGET_S: usize = 20;
pub const CHUNK_TRIGGER_S: usize = 24;

/// RMS → 0..1 для волны и свечения. Порог 0.004 отсекает шум комнаты
/// (тишина должна схлопывать полоски), sqrt — перцептивное сжатие;
/// нормальная речь (RMS 0.02–0.08) даёт 0.5–1.0.
fn perceptual_level(rms: f32) -> f32 {
    ((rms - 0.004).max(0.0) * 14.0).sqrt().min(1.0)
}

fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
