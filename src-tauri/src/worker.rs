//! Поток распознавания. Владеет активным движком — локальной моделью
//! sherpa-onnx или онлайн-API — загружает его по требованию (в т.ч.
//! параллельно с идущей записью) и выгружает локальную модель из памяти
//! после простоя: на 8-гигабайтном Mac это ~1 ГБ, возвращаемый системе.

use serde_json::json;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tauri::AppHandle;

use crate::asr::{self, ModelStatus};
use crate::store::{self, SharedSettings};
use crate::{island, models, online, output, platform};

pub enum WorkerMsg {
    /// Загрузить активный движок, если он ещё не в памяти (старт записи,
    /// запуск приложения).
    EnsureLoaded,
    /// Активная модель сменилась или только что установлена.
    Reload,
    /// Промежуточный кусок записи (стриминг): распознаётся, пока
    /// пользователь ещё говорит.
    Partial {
        session: u64,
        samples: Vec<f32>,
        sample_rate: u32,
    },
    /// Хвост записи: распознать, склеить с накопленными кусками и вставить.
    Final {
        session: u64,
        samples: Vec<f32>,
        sample_rate: u32,
        duration_ms: u64,
    },
    /// Запись отменена — накопленные куски сессии выбросить.
    CancelSession { session: u64 },
}

enum Engine {
    Local(asr::Transcriber),
    Online(online::OnlineEngine),
}

impl Engine {
    fn id(&self) -> String {
        match self {
            Engine::Local(t) => t.model_id.clone(),
            Engine::Online(o) => format!("{}{}", store::ONLINE_PREFIX, o.provider.id),
        }
    }

    /// Ошибка — короткий текст для островка («OpenAI: invalid API key»).
    fn transcribe(&self, samples: &[f32], rate: u32) -> Result<String, String> {
        match self {
            Engine::Local(t) => Ok(t.transcribe_long(samples, rate)),
            Engine::Online(o) => {
                let mut first_err: Option<String> = None;
                let parts = asr::split_long(samples, rate, asr::ONLINE_MAX_CHUNK_S, |piece| match o.transcribe(piece, rate) {
                    Ok(text) => text,
                    Err(e) => {
                        log::error!("онлайн-распознавание: {e:#}");
                        first_err.get_or_insert_with(|| e.to_string());
                        String::new()
                    }
                });
                match first_err {
                    Some(e) if parts.is_empty() => Err(e),
                    _ => Ok(parts.join(" ")),
                }
            }
        }
    }
}

struct Worker {
    app: AppHandle,
    status: ModelStatus,
    settings: SharedSettings,
    engine: Option<Engine>,
    last_used: Instant,
    session: u64,
    parts: Vec<String>,
    /// Первая ошибка онлайн-движка в текущей сессии — показать, если
    /// в итоге ничего не распознано.
    session_error: Option<String>,
}

pub fn spawn(app: AppHandle, rx: Receiver<WorkerMsg>, status: ModelStatus, settings: SharedSettings) {
    std::thread::spawn(move || {
        let mut w = Worker {
            app,
            status,
            settings,
            engine: None,
            last_used: Instant::now(),
            session: 0,
            parts: Vec::new(),
            session_error: None,
        };
        loop {
            let msg = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(msg) => msg,
                Err(RecvTimeoutError::Timeout) => {
                    w.maybe_unload();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            w.last_used = Instant::now();

            // Паника на одном сообщении не должна убивать worker: иначе
            // островок навсегда зависает в «Transcribing…».
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.handle(msg)));
            if outcome.is_err() {
                log::error!("паника в worker распознавания — сессия сброшена");
                w.parts.clear();
                w.session_error = None;
                if w.engine.is_none() {
                    w.emit(asr::STATUS_ERROR, Some("model failed to load".into()));
                }
                island::set_state(&w.app, "error", Some("Something went wrong".into()));
            }
        }
    });
}

impl Worker {
    fn active_id(&self) -> String {
        store::read(&self.settings).active_model.clone()
    }

    fn emit(&self, code: u8, message: Option<String>) {
        asr::emit_status(
            &self.app,
            &self.status,
            code,
            json!({ "model": self.active_id(), "message": message }),
        );
    }

    /// Держит в памяти ровно активный движок. Возвращает false, если
    /// его нельзя подготовить (модель не скачана, нет ключа и т.п.).
    fn ensure_loaded(&mut self) -> bool {
        let active = self.active_id();
        if self.engine.as_ref().is_some_and(|e| e.id() == active) {
            return true;
        }
        // Старую модель выгружаем ДО загрузки новой: две в памяти
        // 8-гигабайтный Mac не потянет.
        self.engine = None;

        if let Some(provider) = active.strip_prefix(store::ONLINE_PREFIX) {
            return match online::OnlineEngine::new(provider) {
                Ok(engine) => {
                    self.engine = Some(Engine::Online(engine));
                    self.emit(asr::STATUS_READY, None);
                    true
                }
                Err(e) => {
                    self.emit(asr::STATUS_ERROR, Some(e.to_string()));
                    false
                }
            };
        }

        let Some(spec) = models::find(&active) else {
            self.emit(asr::STATUS_ERROR, Some(format!("unknown model {active}")));
            return false;
        };
        if !models::is_installed(&self.app, spec) {
            let code = if models::downloads().is_active(&spec.id) {
                asr::STATUS_DOWNLOADING
            } else {
                asr::STATUS_MISSING
            };
            self.emit(code, None);
            return false;
        }

        self.emit(asr::STATUS_LOADING, None);
        match asr::Transcriber::load(spec, &models::model_dir(&self.app, spec)) {
            Ok(t) => {
                let t0 = Instant::now();
                t.warmup();
                log::info!("прогрев {:?}", t0.elapsed());
                self.engine = Some(Engine::Local(t));
                self.emit(asr::STATUS_READY, None);
                true
            }
            Err(e) => {
                log::error!("загрузка модели {active}: {e:#}");
                self.emit(asr::STATUS_ERROR, Some(format!("{e:#}")));
                false
            }
        }
    }

    fn maybe_unload(&mut self) {
        let minutes = store::read(&self.settings).unload_after_min;
        if minutes == 0 || !matches!(self.engine, Some(Engine::Local(_))) {
            return;
        }
        if self.last_used.elapsed() >= Duration::from_secs(minutes as u64 * 60) {
            self.engine = None;
            log::info!("модель выгружена из памяти после {minutes} мин простоя");
            self.emit(asr::STATUS_UNLOADED, None);
        }
    }

    fn begin_session(&mut self, session: u64) {
        if session != self.session {
            self.session = session;
            self.parts.clear();
            self.session_error = None;
        }
    }

    fn handle(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::EnsureLoaded => {
                self.ensure_loaded();
            }
            WorkerMsg::Reload => {
                self.engine = None;
                self.ensure_loaded();
            }
            WorkerMsg::Partial {
                session,
                samples,
                sample_rate,
            } => {
                self.begin_session(session);
                if !self.ensure_loaded() {
                    return;
                }
                let t0 = Instant::now();
                let engine = self.engine.as_ref().expect("ensure_loaded вернул true");
                match engine.transcribe(&samples, sample_rate) {
                    Ok(text) => {
                        log::info!(
                            "промежуточный кусок распознан за {:?}: {} символов",
                            t0.elapsed(),
                            text.chars().count()
                        );
                        if !text.is_empty() {
                            self.parts.push(text);
                        }
                    }
                    Err(e) => {
                        self.session_error.get_or_insert(e);
                    }
                }
            }
            WorkerMsg::Final {
                session,
                samples,
                sample_rate,
                duration_ms,
            } => self.finish(session, samples, sample_rate, duration_ms),
            WorkerMsg::CancelSession { session } => {
                if session == self.session {
                    self.parts.clear();
                    self.session_error = None;
                }
            }
        }
    }

    fn finish(&mut self, session: u64, samples: Vec<f32>, sample_rate: u32, duration_ms: u64) {
        let (sounds, history_keep) = {
            let s = store::read(&self.settings);
            (s.sounds, s.history_keep.clone())
        };
        self.begin_session(session);

        if !self.ensure_loaded() {
            self.parts.clear();
            let message = if self.active_id().starts_with(store::ONLINE_PREFIX) {
                "Online engine unavailable"
            } else {
                "Model is not ready"
            };
            island::set_state(&self.app, "error", Some(message.into()));
            if sounds {
                platform::play(platform::Sound::Error);
            }
            return;
        }

        let t0 = Instant::now();
        let engine = self.engine.as_ref().expect("ensure_loaded вернул true");
        match engine.transcribe(&samples, sample_rate) {
            Ok(tail) if !tail.is_empty() => self.parts.push(tail),
            Ok(_) => {}
            Err(e) => {
                self.session_error.get_or_insert(e);
            }
        }
        let text = self.parts.join(" ").trim().to_string();
        self.parts.clear();
        let error = self.session_error.take();
        log::info!(
            "финал: хвост {} сэмплов за {:?}, всего {} символов (запись {} мс, движок {})",
            samples.len(),
            t0.elapsed(),
            text.chars().count(),
            duration_ms,
            self.active_id()
        );

        if text.is_empty() {
            let message = error.unwrap_or_else(|| "Didn't catch that".into());
            island::set_state(&self.app, "error", Some(message));
            if sounds {
                platform::play(platform::Sound::Error);
            }
            return;
        }
        if let Some(e) = error {
            log::warn!("часть записи не распознана: {e}");
        }

        match output::insert_text(&text) {
            // Текст уже на месте — островок схлопывается сразу.
            Ok(()) => island::set_state(&self.app, "idle", None),
            Err(e) => {
                log::error!("вставка: {e:#}");
                island::set_state(
                    &self.app,
                    "error",
                    Some("Paste failed — text kept in clipboard".into()),
                );
            }
        }

        store::push_history(
            &self.app,
            store::HistoryEntry {
                text,
                ts_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                duration_ms,
            },
            &history_keep,
        );
    }
}
