//! Глобальный хоткей и state machine записи.
//!
//! macOS: собственный listen-only CGEventTap. Критично важные свойства
//! (урок инцидента с зависанием системы из-за rdev):
//!   1. Тап создаётся РОВНО ОДИН РАЗ за жизнь процесса — никаких retry-циклов,
//!      плодящих перехватчики (сотни тапов = системный фриз ввода).
//!   2. Пока нет разрешения Accessibility, тапы вообще не создаются —
//!      просто опрашиваем AXIsProcessTrusted раз в 2 секунды.
//!   3. Источник тапа живёт в run loop СВОЕГО потока; тот же поток
//!      периодически вызывает enable() — самолечение после
//!      kCGEventTapDisabledByTimeout.
//!   4. Колбэк тапа не берёт никаких «чужих» локов: только атомики
//!      и отправка в unbounded-канал.
//!
//! Windows: низкоуровневый хук WH_KEYBOARD_LL на собственном потоке
//! `voxy-hotkey-hook` с приоритетом THREAD_PRIORITY_HIGHEST. Разрешений не
//! нужно. Те же принципы, что у тапа, плюс особенности Win32:
//!   1. В любой момент стоит РОВНО ОДИН хук: переустановка всегда сначала
//!      снимает старый хук, потом ставит новый. Два хука — каждое событие
//!      приходит дважды, и Toggle переключался бы туда и обратно.
//!   2. Колбэк (hook_proc) вызывается внутри GetMessageW потока хука, и ввод
//!      во всей системе ждёт его возврата. Не уложился в LowLevelHooksTimeout
//!      (сотни мс, с Windows 10 1709 не больше 1 с) — Windows МОЛЧА снимает
//!      хук, узнать об этом нельзя. Поэтому в колбэке только атомики и
//!      отправка в unbounded-канал: ни логов, ни локов, ни форматирования, ни
//!      аллокаций сверх send. Вне колбэка поток тоже не делает ничего долгого
//!      (пока он занят, клавиатура всей системы стоит): логи только на
//!      редких путях — старт, ошибки, восстановление.
//!   3. Собственный синтетический ввод Voxy (Ctrl+V от enigo в
//!      output::send_paste, помечен dwExtraInfo = enigo::EVENT_MARKER)
//!      игнорируется: иначе хоткей Left Ctrl запускал бы запись на каждой
//!      вставке. Прочий injected-ввод (флаг LLKHF_INJECTED) НЕ фильтруется:
//!      через него работают ремапы PowerToys и AutoHotkey.
//!   4. Самолечение: раз в 60 с (SetTimer без окна кладёт WM_TIMER в очередь
//!      потока) хук переустанавливается, если ни одна клавиша не зажата —
//!      так не теряется отпускание. Плановая переустановка — единственный
//!      способ, раз о снятии хука система не сообщает.
//!
//! Ограничения Windows для пользователя: тап клавиши Win открывает меню
//! «Пуск» (listen-only хук его не подавит), а левый Alt активирует меню
//! приложений, и его же синтетически нажимает tao, когда выводит окно на
//! передний план (force_window_active). Поэтому настройки предупреждают про
//! Win, а документация советует Right Ctrl или F-клавиши.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use crate::asr::{self, ModelStatus};
use crate::store::{RecordMode, SharedSettings};
use crate::{audio, island, platform, store, WorkerMsg};

pub enum Ctrl {
    Pressed,
    Released,
    /// Достигнута максимальная длительность записи.
    AutoStop,
    /// Накопился готовый кусок — отрезать и отдать на распознавание.
    AutoChunk,
    /// Отмена записи (клик по островку) — аудио отбрасывается.
    Cancel,
    /// Захвачена новая горячая клавиша в настройках.
    SetHotkey(u32),
    CaptureCancelled,
}

/// Порог для режима Dynamic: удержание дольше — работает как PTT,
/// короче — запись «защёлкивается» до следующего нажатия.
const DYNAMIC_HOLD_MS: u128 = 1000;

pub struct HotkeyHandles {
    pub capture: Arc<Capture>,
    pub ctrl_tx: Sender<Ctrl>,
}

/// Сколько ждать клавишу после «Change» в настройках (столько же ждёт
/// страница, CAPTURE_MS в settings.js).
pub const CAPTURE_WINDOW: Duration = Duration::from_secs(15);

/// Захват новой горячей клавиши. Следующее нажатие в системе становится
/// хоткеем, поэтому захват ограничен: он истекает через CAPTURE_WINDOW и
/// срабатывает, только пока окно настроек в фокусе. Иначе скрипт в окне мог
/// бы снова и снова включать захват и так записывать нажатия в других
/// приложениях. Только атомики: состояние читает колбэк тапа.
pub struct Capture {
    epoch: Instant,
    /// До какого момента (мс от epoch) ждём клавишу; 0 — захвата нет.
    until_ms: AtomicU64,
    /// Окно настроек в фокусе (WindowEvent::Focused, см. show_settings).
    settings_focused: AtomicBool,
}

impl Capture {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            until_ms: AtomicU64::new(0),
            settings_focused: AtomicBool::new(false),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub fn begin(&self) {
        let until = self.now_ms() + CAPTURE_WINDOW.as_millis() as u64;
        self.until_ms.store(until, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.until_ms.store(0, Ordering::Relaxed);
    }

    pub fn set_settings_focused(&self, focused: bool) {
        self.settings_focused.store(focused, Ordering::Relaxed);
    }

    /// Для колбэка тапа, на каждое нажатие. None — захвата нет. Иначе
    /// захват снимается, а Some(true) значит, что эта клавиша — новый
    /// хоткей; Some(false) — захват истёк или окно настроек не в фокусе.
    fn take(&self) -> Option<bool> {
        if self.until_ms.load(Ordering::Relaxed) == 0 {
            return None;
        }
        match self.until_ms.swap(0, Ordering::Relaxed) {
            0 => None,
            until => Some(
                self.now_ms() <= until && self.settings_focused.load(Ordering::Relaxed),
            ),
        }
    }
}

/// Имена клавиш ↔ виртуальные коды macOS. Имена совместимы с прежним
/// форматом настроек (стиль rdev), чтобы сохранённые настройки не сломались.
#[cfg(target_os = "macos")]
const KEY_TABLE: &[(&str, u32)] = &[
    ("MetaRight", 0x36),
    ("MetaLeft", 0x37),
    ("ControlLeft", 0x3B),
    ("ControlRight", 0x3E),
    ("ShiftLeft", 0x38),
    ("ShiftRight", 0x3C),
    ("Alt", 0x3A),
    ("AltGr", 0x3D),
    ("CapsLock", 0x39),
    ("Function", 0x3F),
    ("F1", 0x7A),
    ("F2", 0x78),
    ("F3", 0x63),
    ("F4", 0x76),
    ("F5", 0x60),
    ("F6", 0x61),
    ("F7", 0x62),
    ("F8", 0x64),
    ("F9", 0x65),
    ("F10", 0x6D),
    ("F11", 0x67),
    ("F12", 0x6F),
    ("F13", 0x69),
    ("F14", 0x6B),
    ("F15", 0x71),
    ("F16", 0x6A),
    ("F17", 0x40),
    ("F18", 0x4F),
    ("F19", 0x50),
    ("Home", 0x73),
    ("End", 0x77),
    ("PageUp", 0x74),
    ("PageDown", 0x79),
    ("Insert", 0x72),
];

/// Windows: виртуальные коды (VK). Имена совпадают с macOS-таблицей,
/// поэтому settings.json переносим между платформами.
#[cfg(not(target_os = "macos"))]
const KEY_TABLE: &[(&str, u32)] = &[
    ("ControlRight", 0xA3),
    ("ControlLeft", 0xA2),
    ("AltGr", 0xA5), // правый Alt
    ("Alt", 0xA4),   // левый Alt
    ("ShiftRight", 0xA1),
    ("ShiftLeft", 0xA0),
    ("MetaRight", 0x5C), // правый Win
    ("MetaLeft", 0x5B),  // левый Win
    ("CapsLock", 0x14),
    ("F1", 0x70),
    ("F2", 0x71),
    ("F3", 0x72),
    ("F4", 0x73),
    ("F5", 0x74),
    ("F6", 0x75),
    ("F7", 0x76),
    ("F8", 0x77),
    ("F9", 0x78),
    ("F10", 0x79),
    ("F11", 0x7A),
    ("F12", 0x7B),
    ("F13", 0x7C),
    ("F14", 0x7D),
    ("F15", 0x7E),
    ("F16", 0x7F),
    ("F17", 0x80),
    ("F18", 0x81),
    ("F19", 0x82),
    ("Home", 0x24),
    ("End", 0x23),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("Insert", 0x2D),
];

#[cfg(target_os = "macos")]
const ESCAPE_CODE: u32 = 0x35;
#[cfg(not(target_os = "macos"))]
const ESCAPE_CODE: u32 = 0x1B;

pub fn parse_key(name: &str) -> Option<u32> {
    if let Some((_, code)) = KEY_TABLE.iter().find(|(n, _)| *n == name) {
        return Some(*code);
    }
    // Экзотические клавиши сериализуются как "Key0xNN"
    name.strip_prefix("Key0x")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
}

pub fn key_name(code: u32) -> String {
    KEY_TABLE
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| format!("Key0x{code:X}"))
}

pub fn spawn(
    app: AppHandle,
    settings: SharedSettings,
    model_status: ModelStatus,
    worker_tx: Sender<WorkerMsg>,
) -> HotkeyHandles {
    let initial_code = {
        let name = store::read(&settings).hotkey.clone();
        parse_key(&name).unwrap_or_else(|| {
            log::warn!("не удалось разобрать хоткей '{name}', беру по умолчанию");
            parse_key(store::default_hotkey()).unwrap()
        })
    };
    let shared_key = Arc::new(AtomicU32::new(initial_code));
    let capture = Arc::new(Capture::new());
    let (ctrl_tx, ctrl_rx) = channel::<Ctrl>();

    spawn_listener(
        app.clone(),
        shared_key.clone(),
        capture.clone(),
        ctrl_tx.clone(),
    );

    // Контроллер: владеет Recorder (cpal::Stream не Send — не покидает поток).
    let controller_key = shared_key.clone();
    let handles_tx = ctrl_tx.clone();
    std::thread::spawn(move || {
        let mut recorder = audio::Recorder::new();
        let mut recording_since: Option<Instant> = None;
        // Dynamic: началась ли запись текущим (ещё зажатым) нажатием.
        let mut started_by_press = false;
        // Dynamic: нажатие остановило запись — его отпускание игнорируем.
        let mut skip_release = false;
        // Идентификатор сессии записи для стриминговых кусков.
        let mut session: u64 = 0;

        for msg in ctrl_rx {
            // Паника при обработке одного сообщения не должна убивать поток:
            // иначе хоткей молча перестаёт работать до перезапуска.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match msg {
                Ctrl::Pressed => {
                    let mode = store::read(&settings).mode;
                    match (mode, recording_since) {
                        (RecordMode::Ptt | RecordMode::Dynamic, None) => {
                            recording_since = try_start(
                                &app,
                                &settings,
                                &model_status,
                                &mut recorder,
                                ctrl_tx.clone(),
                                &worker_tx,
                            );
                            if recording_since.is_some() {
                                session += 1;
                            }
                            started_by_press = recording_since.is_some();
                        }
                        // Dynamic с «защёлкнутой» записью: повторное нажатие
                        // останавливает сразу (не ждём отпускания).
                        (RecordMode::Dynamic, Some(t0)) if !started_by_press => {
                            finish(&app, &settings, &mut recorder, t0, session, &worker_tx);
                            recording_since = None;
                            skip_release = true;
                        }
                        _ => {}
                    }
                }
                Ctrl::Released if skip_release => {
                    skip_release = false;
                }
                Ctrl::Released => {
                    let mode = store::read(&settings).mode;
                    match (mode, recording_since) {
                        (RecordMode::Ptt, Some(t0)) => {
                            finish(&app, &settings, &mut recorder, t0, session, &worker_tx);
                            recording_since = None;
                        }
                        // Toggle реагирует на отпускание: одно физическое нажатие —
                        // ровно одно событие, без автоповтора.
                        (RecordMode::Toggle, Some(t0)) => {
                            finish(&app, &settings, &mut recorder, t0, session, &worker_tx);
                            recording_since = None;
                        }
                        (RecordMode::Toggle, None) => {
                            recording_since = try_start(
                                &app,
                                &settings,
                                &model_status,
                                &mut recorder,
                                ctrl_tx.clone(),
                                &worker_tx,
                            );
                            if recording_since.is_some() {
                                session += 1;
                            }
                        }
                        (RecordMode::Dynamic, Some(t0)) if started_by_press => {
                            started_by_press = false;
                            if t0.elapsed().as_millis() >= DYNAMIC_HOLD_MS {
                                // Долгое удержание = классический PTT.
                                finish(&app, &settings, &mut recorder, t0, session, &worker_tx);
                                recording_since = None;
                            }
                            // Короткий тап — запись «защёлкнута», продолжаем.
                        }
                        _ => {}
                    }
                }
                Ctrl::AutoStop => {
                    if let Some(t0) = recording_since {
                        finish(&app, &settings, &mut recorder, t0, session, &worker_tx);
                        recording_since = None;
                        started_by_press = false;
                    }
                }
                Ctrl::AutoChunk => {
                    if recording_since.is_some() {
                        if let Some((samples, sample_rate)) =
                            recorder.take_chunk(audio::CHUNK_TARGET_S)
                        {
                            log::info!(
                                "стриминг: кусок {:.1} c ушёл на распознавание",
                                samples.len() as f32 / sample_rate as f32
                            );
                            let _ = worker_tx.send(WorkerMsg::Partial {
                                session,
                                samples,
                                sample_rate,
                            });
                        }
                    }
                }
                Ctrl::Cancel => {
                    if recording_since.is_some() {
                        let _ = recorder.stop(); // аудио отбрасываем
                        recording_since = None;
                        started_by_press = false;
                        let _ = worker_tx.send(WorkerMsg::CancelSession { session });
                        island::set_state(&app, "cancelled", None);
                        island::set_interactive(&app, false);
                    }
                }
                Ctrl::SetHotkey(code) => {
                    controller_key.store(code, Ordering::Relaxed);
                    let name = key_name(code);
                    {
                        let mut s = store::write(&settings);
                        s.hotkey = name.clone();
                        store::save_settings(&app, &s);
                    }
                    let _ = app.emit_to(
                        "settings",
                        "hotkey-captured",
                        serde_json::json!({ "key": name }),
                    );
                }
                Ctrl::CaptureCancelled => {
                    let _ = app.emit_to(
                        "settings",
                        "hotkey-captured",
                        serde_json::json!({ "cancelled": true }),
                    );
                }
            }
            }));

            if outcome.is_err() {
                log::error!("паника в контроллере записи — состояние сброшено");
                let _ = recorder.stop();
                if recording_since.take().is_some() {
                    let _ = worker_tx.send(WorkerMsg::CancelSession { session });
                }
                started_by_press = false;
                skip_release = false;
                island::set_interactive(&app, false);
                island::set_state(&app, "error", Some("Something went wrong".into()));
            }
        }
    });

    HotkeyHandles {
        capture,
        ctrl_tx: handles_tx,
    }
}

fn try_start(
    app: &AppHandle,
    settings: &SharedSettings,
    model_status: &ModelStatus,
    recorder: &mut audio::Recorder,
    ctrl_tx: Sender<Ctrl>,
    worker_tx: &Sender<WorkerMsg>,
) -> Option<Instant> {
    let (sounds, device, max_s) = {
        let s = store::read(settings);
        (s.sounds, s.mic_device.clone(), s.max_record_s)
    };

    // Выгруженная после простоя или ещё загружающаяся модель запись не
    // блокирует: worker загрузит её, пока человек говорит.
    if !asr::can_record(model_status.load(Ordering::Relaxed)) {
        island::set_state(app, "error", Some("Model is not ready yet".into()));
        if sounds {
            platform::play(platform::Sound::Error);
        }
        return None;
    }

    match recorder.start(app, device.as_deref(), max_s, ctrl_tx) {
        Ok(()) => {
            let _ = worker_tx.send(WorkerMsg::EnsureLoaded);
            if sounds {
                platform::play(platform::Sound::Start);
            }
            island::set_recording(app, max_s);
            // Во время записи островок принимает клики (кнопка Cancel).
            island::set_interactive(app, true);
            Some(Instant::now())
        }
        Err(e) => {
            log::error!("не удалось начать запись: {e:#}");
            island::set_state(app, "error", Some("Microphone unavailable".into()));
            if sounds {
                platform::play(platform::Sound::Error);
            }
            None
        }
    }
}

fn finish(
    app: &AppHandle,
    settings: &SharedSettings,
    recorder: &mut audio::Recorder,
    t0: Instant,
    session: u64,
    worker_tx: &Sender<WorkerMsg>,
) {
    let (samples, sample_rate) = recorder.stop();
    island::set_interactive(app, false);
    let duration_ms = t0.elapsed().as_millis() as u64;
    let (sounds, min_ms) = {
        let s = store::read(settings);
        (s.sounds, s.min_duration_ms)
    };

    if duration_ms < min_ms {
        // Случайное касание — тихо схлопываем островок.
        let _ = worker_tx.send(WorkerMsg::CancelSession { session });
        island::set_state(app, "cancelled", None);
        return;
    }

    if sounds {
        platform::play(platform::Sound::Stop);
    }
    island::set_state(app, "processing", None);
    let _ = worker_tx.send(WorkerMsg::Final {
        session,
        samples,
        sample_rate,
        duration_ms,
    });
}

// ------------------------------------------------------------- слушатель

/// Обработка одного нажатия/отпускания. Вызывается из колбэка тапа —
/// никаких локов, только атомики и канал.
fn on_press(code: u32, shared_key: &AtomicU32, capture: &Capture, ctrl_tx: &Sender<Ctrl>) {
    match capture.take() {
        Some(true) => {
            if code == ESCAPE_CODE {
                let _ = ctrl_tx.send(Ctrl::CaptureCancelled);
            } else {
                let _ = ctrl_tx.send(Ctrl::SetHotkey(code));
            }
            return;
        }
        // Захват истёк или окно настроек не в фокусе: страница узнаёт об
        // отмене, а клавиша обрабатывается как обычно.
        Some(false) => {
            let _ = ctrl_tx.send(Ctrl::CaptureCancelled);
        }
        None => {}
    }
    if code == shared_key.load(Ordering::Relaxed) {
        let _ = ctrl_tx.send(Ctrl::Pressed);
    }
}

fn on_release(code: u32, shared_key: &AtomicU32, ctrl_tx: &Sender<Ctrl>) {
    if code == shared_key.load(Ordering::Relaxed) {
        let _ = ctrl_tx.send(Ctrl::Released);
    }
}

/// Решения хука Windows без вызовов ОС. Компилируются на всех платформах,
/// чтобы юнит-тесты шли в CI и на macOS, и на Windows; вне Windows в
/// обычной сборке не используются.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod win_hook {
    use super::{AtomicU64, Ordering};

    /// Метка, которой enigo помечает каждое своё синтетическое событие
    /// (KBDLLHOOKSTRUCT.dwExtraInfo). output.rs создаёт Enigo с настройками
    /// по умолчанию (windows_dw_extra_info = None), поэтому Ctrl+V от
    /// вставки приходит именно с этой меткой.
    const OWN_INPUT_MARKER: usize = enigo::EVENT_MARKER as usize;

    /// Событие — собственный синтетический ввод Voxy? Такие события хук
    /// пропускает мимо state machine. Прочий injected-ввод (LLKHF_INJECTED:
    /// PowerToys, AutoHotkey, Alt-тап tao) обрабатывается как обычный.
    pub(super) fn is_own_input(extra_info: usize) -> bool {
        extra_info == OWN_INPUT_MARKER
    }

    /// Слово и бит виртуального кода в маске зажатых клавиш
    /// (VK < 256: четыре слова по 64 бита).
    pub(super) fn down_slot(vk: u32) -> (usize, u64) {
        (((vk / 64) as usize).min(3), 1u64 << (vk % 64))
    }

    /// Ни одна клавиша не зажата: переустановка хука не потеряет отпускание.
    pub(super) fn no_key_held(words: &[AtomicU64; 4]) -> bool {
        words.iter().all(|w| w.load(Ordering::Relaxed) == 0)
    }

    /// Сколько тиков подряд плановая переустановка ждёт отпускания клавиш.
    /// Отпускание, ушедшее на защищённый рабочий стол (Win+L, Ctrl+Alt+Del,
    /// запрос UAC), хук не видит, и бит клавиши залипает до следующего её
    /// нажатия; без предела залипший бит навсегда выключил бы самолечение.
    /// 10 тиков (10 минут) — дольше самой длинной записи (300 с). А реально
    /// зажатая клавиша переустановку переживает: её отпускание теряется,
    /// только если попадёт в микросекундный зазор между снятием и установкой.
    pub(super) const MAX_REHOOK_DEFERRALS: u32 = 10;

    /// Пора ли переустановить хук на этом тике таймера.
    pub(super) fn rehook_due(no_key_held: bool, deferred_ticks: u32) -> bool {
        no_key_held || deferred_ticks >= MAX_REHOOK_DEFERRALS
    }
}

#[cfg(target_os = "macos")]
fn spawn_listener(
    app: AppHandle,
    shared_key: Arc<AtomicU32>,
    capture: Arc<Capture>,
    ctrl_tx: Sender<Ctrl>,
) {
    use core_foundation::base::TCFType;
    use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult, EventField,
    };

    std::thread::spawn(move || {
        // Системные диалоги Accessibility + Input Monitoring: регистрируют
        // приложение в списках TCC (в т.ч. заново после пересборки).
        platform::request_permissions_prompt();

        // Ждём ОБА разрешения, НЕ создавая тапов; статус шлём в настройки
        // при каждом изменении — баннер обновляется живьём.
        let mut last_status: Option<(bool, bool)> = None;
        loop {
            let acc = platform::accessibility_trusted();
            let im = platform::input_monitoring_granted();
            if last_status != Some((acc, im)) {
                last_status = Some((acc, im));
                log::info!("разрешения: accessibility={acc} input_monitoring={im}");
                let _ = app.emit_to(
                    "settings",
                    "permissions",
                    serde_json::json!({ "accessibility": acc, "input_monitoring": im }),
                );
            }
            if acc && im {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }

        // Создание тапа с ретраем. Безопасно: НЕУДАЧА ничего не создаёт
        // (CGEventTapCreate вернул null), а после УСПЕХА цикл завершается —
        // гарантия «не больше одного тапа на процесс» сохраняется.
        // Колбэк не берёт локов: паника/блокировка внутри него уходит в C-код.
        let tap = loop {
            let shared_key = shared_key.clone();
            let capture = capture.clone();
            let ctrl_tx = ctrl_tx.clone();
            let result = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![
                    CGEventType::KeyDown,
                    CGEventType::KeyUp,
                    CGEventType::FlagsChanged,
                ],
                move |_proxy, etype, event| {
                    // macOS выключает тап, если колбэк тормозил или включён
                    // secure input. Включаем обратно сразу; событие здесь
                    // может быть пустым — к его полям не обращаемся.
                    if matches!(
                        etype,
                        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                    ) {
                        let port = TAP_PORT.load(Ordering::Acquire);
                        if !port.is_null() {
                            unsafe { CGEventTapEnable(port, true) };
                        }
                        return CallbackResult::Keep;
                    }

                    let code =
                        event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u32;
                    match etype {
                        CGEventType::KeyDown => {
                            let repeat = event
                                .get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT);
                            if repeat == 0 {
                                on_press(code, &shared_key, &capture, &ctrl_tx);
                            }
                        }
                        CGEventType::KeyUp => on_release(code, &shared_key, &ctrl_tx),
                        CGEventType::FlagsChanged => {
                            let down = match modifier_mask(code) {
                                Some(mask) => event.get_flags().bits() & mask != 0,
                                None => toggle_unknown_modifier(code),
                            };
                            if down {
                                on_press(code, &shared_key, &capture, &ctrl_tx);
                            } else {
                                on_release(code, &shared_key, &ctrl_tx);
                            }
                        }
                        _ => {}
                    }
                    CallbackResult::Keep
                },
            );
            match result {
                Ok(tap) => break tap,
                Err(()) => {
                    log::warn!("event tap не создался, повтор через 5 с");
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        };

        let source = match tap.mach_port().create_runloop_source(0) {
            Ok(s) => s,
            Err(_) => {
                log::error!("не удалось создать run loop source для тапа");
                return;
            }
        };
        TAP_PORT.store(
            tap.mach_port().as_concrete_TypeRef() as *mut std::ffi::c_void,
            Ordering::Release,
        );
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopDefaultMode });
        tap.enable();
        log::info!("глобальный хоткей активен (event tap установлен)");

        // Run loop этого потока обслуживает тап. Выключение тапа чинится
        // мгновенно в колбэке; раз в 30 с — страховочный enable().
        loop {
            CFRunLoop::run_in_mode(
                unsafe { kCFRunLoopDefaultMode },
                Duration::from_secs(30),
                false,
            );
            tap.enable();
        }
    });
}

/// Mach-порт единственного тапа — чтобы колбэк мог включить его обратно.
#[cfg(target_os = "macos")]
static TAP_PORT: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapEnable(tap: *mut std::ffi::c_void, enable: bool);
}

/// Бит модификатора в флагах события (device-dependent маски из
/// IOLLEvent.h). По нему состояние клавиши читается, а не угадывается:
/// пропущенное событие больше не инвертирует push-to-talk.
#[cfg(target_os = "macos")]
fn modifier_mask(code: u32) -> Option<u64> {
    Some(match code {
        0x3B => 0x0000_0001, // левый Control
        0x38 => 0x0000_0002, // левый Shift
        0x3C => 0x0000_0004, // правый Shift
        0x37 => 0x0000_0008, // левый Command
        0x36 => 0x0000_0010, // правый Command
        0x3A => 0x0000_0020, // левый Option
        0x3D => 0x0000_0040, // правый Option
        0x3E => 0x0000_2000, // правый Control
        0x39 => 0x0001_0000, // Caps Lock (alphaShift)
        0x3F => 0x0080_0000, // Fn (secondaryFn)
        _ => return None,
    })
}

/// Запасной путь для модификаторов без известного бита: переключатель
/// в атомарной маске. Возвращает true, если клавиша теперь нажата.
#[cfg(target_os = "macos")]
fn toggle_unknown_modifier(code: u32) -> bool {
    static DOWN: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    let bit = 1u64 << (code % 64);
    let prev = DOWN[((code / 64) as usize).min(1)].fetch_xor(bit, Ordering::Relaxed);
    prev & bit == 0
}

/// Windows: низкоуровневый клавиатурный хук WH_KEYBOARD_LL на собственном
/// потоке (инварианты — в документации модуля). Колбэк вызывается внутри
/// GetMessageW этого потока; тот же цикл сообщений раз в минуту
/// переустанавливает хук, если Windows молча его сняла.
#[cfg(target_os = "windows")]
fn spawn_listener(
    _app: AppHandle,
    shared_key: Arc<AtomicU32>,
    capture: Arc<Capture>,
    ctrl_tx: Sender<Ctrl>,
) {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_INVALID_HOOK_HANDLE, HMODULE, LPARAM, LRESULT, WPARAM,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, SetTimer, SetWindowsHookExW,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
    };

    /// Период плановой переустановки хука. Снятие хука по таймауту бывает
    /// только под тяжёлой нагрузкой, так что минута ограничивает время
    /// «мёртвого» хоткея, а цена — два дешёвых системных вызова в минуту.
    /// Чаще не нужно: каждая переустановка открывает микросекундный зазор,
    /// в котором событие может пройти мимо хука.
    const REHOOK_INTERVAL_MS: u32 = 60_000;

    struct HookCtx {
        shared_key: Arc<AtomicU32>,
        capture: Arc<Capture>,
        ctrl_tx: Sender<Ctrl>,
        /// Зажатые клавиши (VK < 256) атомарной маской: WM_KEYDOWN
        /// автоповторяется, press шлём один раз. Без локов в колбэке хука.
        down: [AtomicU64; 4],
    }
    static CTX: OnceLock<HookCtx> = OnceLock::new();
    if CTX
        .set(HookCtx {
            shared_key,
            capture,
            ctrl_tx,
            down: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        })
        .is_err()
    {
        log::error!("слушатель клавиатуры уже запущен");
        return;
    }

    /// Колбэк WH_KEYBOARD_LL. Выполняется на потоке хука внутри GetMessageW,
    /// и ввод всей системы ждёт его возврата: дольше LowLevelHooksTimeout —
    /// и Windows молча снимает хук. Поэтому здесь только атомики и send в
    /// unbounded-канал: никаких логов, Mutex, форматирования строк и
    /// аллокаций сверх send. Хук listen-only: событие всегда уходит дальше
    /// по цепочке, ничего не проглатываем.
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // code < 0 (не HC_ACTION): по контракту сразу передаём дальше.
        if code >= 0 {
            if let Some(ctx) = CTX.get() {
                let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
                // Ctrl+V, который вставляет сам Voxy, — не нажатие человека:
                // ни state machine, ни маска зажатых клавиш его не видят.
                if !win_hook::is_own_input(kb.dwExtraInfo) {
                    let vk = kb.vkCode;
                    let (index, bit) = win_hook::down_slot(vk);
                    let word = &ctx.down[index];
                    match wparam as u32 {
                        WM_KEYDOWN | WM_SYSKEYDOWN => {
                            if word.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
                                on_press(vk, &ctx.shared_key, &ctx.capture, &ctx.ctrl_tx);
                            }
                        }
                        WM_KEYUP | WM_SYSKEYUP => {
                            word.fetch_and(!bit, Ordering::Relaxed);
                            on_release(vk, &ctx.shared_key, &ctx.ctrl_tx);
                        }
                        _ => {}
                    }
                }
            }
        }
        // Первый аргумент CallNextHookEx игнорируется, поэтому HHOOK колбэку
        // не нужен и живёт локальной переменной потока хука.
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// (Пере)установка хука на потоке хука: снять `old` (если есть), затем
    /// поставить новый — двух хуков одновременно не бывает. Возвращает
    /// текущий хук; null — хука нет, следующий тик таймера попробует снова.
    /// Логирует только редкие события; плановая замена — на уровне debug.
    unsafe fn reinstall_hook(old: HHOOK, module: HMODULE) -> HHOOK {
        // Работающего хука до этого вызова не было: успех стоит показать в логе.
        let mut was_lost = old.is_null();
        if !old.is_null() && UnhookWindowsHookEx(old) == 0 {
            let err = GetLastError();
            if err != ERROR_INVALID_HOOK_HANDLE {
                // Хук, возможно, всё ещё стоит: второй дал бы каждое
                // событие дважды. Оставляем старый, пробуем на следующем тике.
                log::error!(
                    "не удалось снять клавиатурный хук (GetLastError={err}), повтор через минуту"
                );
                return old;
            }
            // Хендл уже недействителен — система сама сняла хук (скорее
            // всего, колбэк не уложился в LowLevelHooksTimeout).
            log::warn!("клавиатурный хук был снят системой, ставлю заново");
            was_lost = true;
        }
        // hMod: контракт SetWindowsHookEx с dwThreadId = 0 требует модуль,
        // в котором лежит колбэк (это exe).
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
        if hook.is_null() {
            // Повтор — на следующем тике таймера (если таймер создан).
            let err = GetLastError();
            log::error!("не удалось установить клавиатурный хук (GetLastError={err})");
        } else if was_lost {
            // Подстрока «хук установлен» — контракт со smoke-тестом CI.
            log::info!("глобальный хоткей активен (клавиатурный хук установлен)");
        } else {
            log::debug!("клавиатурный хук переустановлен (плановая проверка)");
        }
        hook
    }

    let spawned = std::thread::Builder::new()
        .name("voxy-hotkey-hook".into())
        .spawn(|| unsafe {
            // Колбэк выполняется только когда этот поток получает квант.
            // Пока распознавание грузит все ядра, поток с обычным приоритетом
            // может не успеть за LowLevelHooksTimeout — и хук пропадёт.
            if SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) == 0 {
                let err = GetLastError();
                log::warn!("не удалось поднять приоритет потока хука (GetLastError={err})");
            }
            let Some(ctx) = CTX.get() else {
                return;
            };
            let module = GetModuleHandleW(std::ptr::null());
            let mut hook = reinstall_hook(std::ptr::null_mut(), module);

            // Таймер без окна: WM_TIMER приходит в очередь этого потока
            // (msg.hwnd == null, wParam == id таймера).
            let timer = SetTimer(std::ptr::null_mut(), 0, REHOOK_INTERVAL_MS, None);
            if timer == 0 {
                let err = GetLastError();
                log::warn!(
                    "таймер переустановки хука не создан (GetLastError={err}): самолечение выключено"
                );
                if hook.is_null() {
                    return; // ни хука, ни повторов: ошибка уже в логе
                }
            }

            // Сколько тиков подряд переустановка ждала отпускания клавиш.
            let mut deferred: u32 = 0;
            let mut msg: MSG = std::mem::zeroed();
            loop {
                // 0 — WM_QUIT, -1 — ошибка; иначе пришло сообщение.
                match GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) {
                    0 => {
                        log::error!(
                            "цикл сообщений хука завершён (WM_QUIT): хоткей выключен до перезапуска"
                        );
                        break;
                    }
                    -1 => {
                        let err = GetLastError();
                        log::error!(
                            "цикл сообщений хука завершён (GetLastError={err}): хоткей выключен до перезапуска"
                        );
                        break;
                    }
                    _ => {}
                }
                if msg.message == WM_TIMER
                    && msg.hwnd.is_null()
                    && timer != 0
                    && msg.wParam == timer
                {
                    if win_hook::rehook_due(win_hook::no_key_held(&ctx.down), deferred) {
                        deferred = 0;
                        hook = reinstall_hook(hook, module);
                    } else {
                        deferred += 1;
                    }
                    continue;
                }
                // Окон на этом потоке нет; диспетчеризация безвредна и
                // ничего не сломает, если система что-то сюда пришлёт.
                DispatchMessageW(&msg);
            }

            if timer != 0 {
                KillTimer(std::ptr::null_mut(), timer);
            }
            if !hook.is_null() {
                UnhookWindowsHookEx(hook);
            }
        });
    if let Err(e) = spawned {
        log::error!("не удалось запустить поток клавиатурного хука: {e}");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn spawn_listener(
    _app: AppHandle,
    _shared_key: Arc<AtomicU32>,
    _capture: Arc<Capture>,
    _ctrl_tx: Sender<Ctrl>,
) {
    log::warn!("глобальный хоткей не реализован для этой платформы");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Часы захвата, сдвинутые на минуту назад: now_ms() ≈ 60 000.
    fn capture() -> Capture {
        let mut c = Capture::new();
        c.epoch = Instant::now()
            .checked_sub(Duration::from_secs(60))
            .expect("uptime > 60 s");
        c
    }

    fn press(capture: &Capture, code: u32) -> Vec<&'static str> {
        let (tx, rx) = channel();
        on_press(code, &AtomicU32::new(0x36), capture, &tx);
        drop(tx);
        rx.iter()
            .map(|m| match m {
                Ctrl::Pressed => "pressed",
                Ctrl::SetHotkey(_) => "set",
                Ctrl::CaptureCancelled => "cancelled",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn capture_takes_one_key_while_settings_focused() {
        let c = capture();
        c.set_settings_focused(true);
        assert_eq!(press(&c, 0x3E), Vec::<&str>::new()); // захвата нет
        c.begin();
        assert_eq!(press(&c, 0x3E), ["set"]);
        // захват одноразовый: следующая клавиша — обычное нажатие
        assert_eq!(press(&c, 0x36), ["pressed"]);
        c.begin();
        assert_eq!(press(&c, ESCAPE_CODE), ["cancelled"]);
        c.begin();
        c.cancel();
        assert_eq!(press(&c, 0x3E), Vec::<&str>::new());
    }

    /// Нажатия в других приложениях и после срока не становятся хоткеем.
    #[test]
    fn capture_rejects_unfocused_and_expired() {
        let c = capture();
        c.begin();
        assert_eq!(press(&c, 0x00), ["cancelled"]); // окно не в фокусе
        assert_eq!(press(&c, 0x00), Vec::<&str>::new()); // и захват снят

        c.set_settings_focused(true);
        c.until_ms.store(c.now_ms() - 1, Ordering::Relaxed); // срок вышел
        assert_eq!(press(&c, 0x36), ["cancelled", "pressed"]);
        c.begin();
        c.set_settings_focused(false);
        assert_eq!(press(&c, 0x00), ["cancelled"]);
    }

    /// Хук Windows пропускает только Ctrl+V самого Voxy (метка enigo);
    /// физические клавиши и чужой injected-ввод (dwExtraInfo 0 у Alt-тапа
    /// tao, свои значения у ремаперов) остаются нажатиями.
    #[test]
    fn hook_ignores_only_own_injected_input() {
        assert!(win_hook::is_own_input(enigo::EVENT_MARKER as usize));
        assert!(win_hook::is_own_input(100));
        assert!(!win_hook::is_own_input(0));
        assert!(!win_hook::is_own_input(101));
        assert!(!win_hook::is_own_input(usize::MAX));
    }

    /// Плановая переустановка хука ждёт, пока все клавиши отпущены, но не
    /// бесконечно: залипший бит не должен навсегда выключить самолечение.
    #[test]
    fn rehook_waits_for_released_keys_but_not_forever() {
        let down: [AtomicU64; 4] = Default::default();
        assert!(win_hook::no_key_held(&down));

        // Right Ctrl (VK 0xA3) зажат: третье слово, бит 35.
        let (index, bit) = win_hook::down_slot(0xA3);
        assert_eq!((index, bit), (2, 1u64 << 35));
        down[index].fetch_or(bit, Ordering::Relaxed);
        assert!(!win_hook::no_key_held(&down));
        // Крайние коды попадают в маску, а не за её пределы.
        assert_eq!(win_hook::down_slot(0x00), (0, 1));
        assert_eq!(win_hook::down_slot(0xFF), (3, 1u64 << 63));

        assert!(!win_hook::rehook_due(false, 0));
        assert!(!win_hook::rehook_due(
            false,
            win_hook::MAX_REHOOK_DEFERRALS - 1
        ));
        assert!(win_hook::rehook_due(false, win_hook::MAX_REHOOK_DEFERRALS));
        assert!(win_hook::rehook_due(true, 0));

        down[index].fetch_and(!bit, Ordering::Relaxed);
        assert!(win_hook::no_key_held(&down));
    }
}
