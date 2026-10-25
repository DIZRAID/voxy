//! Платформенный шов: звуки, разрешения, клавиши вставки, буфер обмена
//! (поколение и пометка «секрет»), диагностика вставки в окно
//! администратора (UIPI, только Windows). Платформенные различия живут
//! в реализациях этого файла (плюс горячая клавиша по умолчанию в store.rs).

use std::sync::OnceLock;
use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Вызывается один раз в setup: звукам нужен главный поток AppKit.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

#[derive(Clone, Copy)]
pub enum Sound {
    Start,
    Stop,
    Error,
}

/// Системный звук внутри процесса. Раньше это был `afplay` — отдельный
/// процесс на каждый звук, который к тому же никто не дожидался
/// (копились zombie-процессы).
#[cfg(target_os = "macos")]
pub fn play(sound: Sound) {
    let name = match sound {
        Sound::Start => "Pop",
        Sound::Stop => "Bottle",
        Sound::Error => "Basso",
    };
    let Some(app) = APP.get() else { return };
    let _ = app.run_on_main_thread(move || {
        use objc2_app_kit::NSSound;
        use objc2_foundation::NSString;
        // soundNamed возвращает общий кешированный экземпляр: при быстрых
        // повторах его нужно остановить, иначе play() вернёт false.
        if let Some(s) = NSSound::soundNamed(&NSString::from_str(name)) {
            s.stop();
            s.play();
        }
    });
}

/// Звуки Windows: те же роли, что Pop / Bottle / Basso на macOS. Файлы Apple
/// можно использовать только на технике Apple, поэтому здесь свои
/// синтезированные двойники, похожие по характеру (src-tauri/sounds,
/// генератор make-sounds.py рядом). Раньше играли псевдонимы звуковой схемы
/// Windows (SystemAsterisk и т. п.) — звучало совсем не как на Mac.
const START_WAV: &[u8] = include_bytes!("../sounds/start.wav");
const STOP_WAV: &[u8] = include_bytes!("../sounds/stop.wav");
const ERROR_WAV: &[u8] = include_bytes!("../sounds/error.wav");

/// WAV-файл звука; на всех платформах, чтобы тесты проверяли файлы и на macOS.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn sound_wav(sound: Sound) -> &'static [u8] {
    match sound {
        Sound::Start => START_WAV,
        Sound::Stop => STOP_WAV,
        Sound::Error => ERROR_WAV,
    }
}

/// Встроенный WAV из памяти процесса (SND_MEMORY).
///
/// С SND_ASYNC PlaySoundW возвращается сразу и читает данные во время
/// проигрывания, поэтому буфер обязан жить до конца звука: include_bytes!
/// лежит в образе программы всё время работы. Новый вызов прерывает
/// предыдущий звук (у процесса один канал PlaySound), как и раньше.
/// SND_NODEFAULT: если звук не проигрался, молчим, а не «дзынь» Windows.
/// Громкость — общая громкость системы; выключаются звуки переключателем
/// Sounds в настройках Voxy (звуковая схема Windows на них не влияет).
#[cfg(target_os = "windows")]
pub fn play(sound: Sound) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    let wav = sound_wav(sound);
    // SAFETY: с SND_MEMORY первый аргумент — указатель на образ WAV-файла в
    // памяти (тип PCWSTR формальный, данные читаются как байты, выравнивание
    // не требуется); буфер статический и живёт всё время работы программы;
    // hmod = NULL допустим. Результат (BOOL) не нужен: звук — необязательная
    // подсказка.
    unsafe {
        PlaySoundW(
            wav.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn play(_sound: Sound) {}

/// Модификатор для вставки: Cmd на macOS, Ctrl на Windows.
pub fn paste_modifier() -> enigo::Key {
    #[cfg(target_os = "macos")]
    {
        enigo::Key::Meta
    }
    #[cfg(not(target_os = "macos"))]
    {
        enigo::Key::Control
    }
}

/// Клавиша V по виртуальному коду — работает при любой раскладке
/// (включая русскую), в отличие от Key::Unicode('v').
pub fn v_key() -> enigo::Key {
    #[cfg(target_os = "macos")]
    {
        enigo::Key::Other(9) // kVK_ANSI_V
    }
    #[cfg(target_os = "windows")]
    {
        enigo::Key::Other(0x56) // VK_V
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        enigo::Key::Unicode('v')
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(
        options: core_foundation::dictionary::CFDictionaryRef,
    ) -> bool;
    static kAXTrustedCheckOptionPrompt: core_foundation::string::CFStringRef;
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// request: 0 = PostEvent, 1 = ListenEvent (Input Monitoring)
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[cfg(target_os = "macos")]
const KIOHID_REQUEST_LISTEN_EVENT: u32 = 1;

/// Есть ли у процесса разрешение Accessibility (нужно enigo для ⌘V).
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        AXIsProcessTrusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Есть ли Input Monitoring (нужно listen-only CGEventTap для хоткея).
pub fn input_monitoring_granted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        IOHIDCheckAccess(KIOHID_REQUEST_LISTEN_EVENT) == 0 // kIOHIDAccessTypeGranted
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Показывает системные диалоги запроса Accessibility и Input Monitoring.
/// Регистрирует приложение в обоих списках TCC — в том числе ЗАНОВО после
/// пересборки, когда старая запись «протухает» из-за смены подписи бинарника.
#[cfg(target_os = "macos")]
pub fn request_permissions_prompt() {
    unsafe {
        use core_foundation::base::TCFType;
        use core_foundation::boolean::CFBoolean;
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::string::CFString;

        if !AXIsProcessTrusted() {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let options =
                CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
            let _ = AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
        }
        if IOHIDCheckAccess(KIOHID_REQUEST_LISTEN_EVENT) != 0 {
            let _ = IOHIDRequestAccess(KIOHID_REQUEST_LISTEN_EVENT);
        }
    }
}

/// Открывает URL/страницу системных настроек системной утилитой.
/// Процесс дожидаемся в отдельном потоке, иначе он остаётся zombie.
pub fn open_external(target: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    let target = target.to_string();
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new(cmd).arg(&target).status() {
            log::warn!("не удалось открыть {target}: {e}");
        }
    });
}

pub fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    open_external("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
}

pub fn open_input_monitoring_settings() {
    #[cfg(target_os = "macos")]
    open_external("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
}

/// Сколько потоков отдавать нейросети: число производительных ядер
/// (P-cores), не больше 4. Потоки на энергоэффективных ядрах только
/// тормозят остальных (те ждут их в спин-цикле), а на слабых чипах
/// с 2 P-ядрами жёсткие 4 потока перегружали систему.
pub fn inference_threads() -> i32 {
    #[cfg(target_os = "macos")]
    {
        let mut value: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        let name = c"hw.perflevel0.physicalcpu";
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut value as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 && value > 0 {
            return value.clamp(1, 4);
        }
    }
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4) as i32)
        .unwrap_or(2)
}

/// Поколение буфера обмена: растёт при каждой новой записи в буфер, кто бы
/// её ни сделал. macOS — NSPasteboard.changeCount, Windows —
/// GetClipboardSequenceNumber. None — платформа не сообщает; тогда
/// output::insert_text не может заметить чужую запись и восстанавливает
/// буфер всегда.
#[cfg(target_os = "macos")]
pub fn clipboard_generation() -> Option<isize> {
    // Вызывается с рабочего потока: без пула автоосвобождения временные
    // объекты AppKit на нём бы копились.
    objc2::rc::autoreleasepool(|_| {
        Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount())
    })
}

/// Счётчик ведётся системой на каждую window station и увеличивается при
/// каждом изменении или очистке буфера. Открывать буфер (OpenClipboard)
/// не нужно. 0 означает «нет права WINSTA_ACCESSCLIPBOARD» — это не
/// поколение, поэтому None.
#[cfg(target_os = "windows")]
pub fn clipboard_generation() -> Option<isize> {
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    // SAFETY: функция без аргументов, только читает счётчик window station.
    match unsafe { GetClipboardSequenceNumber() } {
        0 => None,
        n => Some(n as isize),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn clipboard_generation() -> Option<isize> {
    None
}

/// В буфере секрет: менеджер паролей пометил содержимое как скрытое или
/// временное (org.nspasteboard.ConcealedType / TransientType, стандарт
/// nspasteboard.org). Такое содержимое Voxy после вставки не возвращает:
/// вернулось бы уже без пометки, и его записала бы история буфера обмена.
#[cfg(target_os = "macos")]
pub fn clipboard_holds_secret() -> bool {
    objc2::rc::autoreleasepool(|_| {
        pasteboard_holds_secret(&objc2_app_kit::NSPasteboard::generalPasteboard())
    })
}

#[cfg(target_os = "macos")]
fn pasteboard_holds_secret(pasteboard: &objc2_app_kit::NSPasteboard) -> bool {
    const MARKERS: [&str; 2] = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];
    pasteboard.types().is_some_and(|types| {
        types.to_vec().iter().any(|t| MARKERS.contains(&t.to_string().as_str()))
    })
}

/// Windows: менеджеры паролей (1Password, Bitwarden, KeePassXC) кладут
/// рядом с секретом формат «ExcludeClipboardContentFromMonitorProcessing»
/// — договорённость Windows для «мониторы буфера, не трогайте это».
/// Собственные записи Voxy его не ставят: arboard в exclude_from_history()
/// пишет только CanIncludeInClipboardHistory = 0 (журнал Win+V), поэтому
/// продиктованный ранее и возвращённый в буфер текст за секрет не считается.
/// RegisterClipboardFormatW и IsClipboardFormatAvailable работают без
/// OpenClipboard, так что проверка не мешает чужим операциям с буфером.
#[cfg(target_os = "windows")]
pub fn clipboard_holds_secret() -> bool {
    use windows_sys::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    // Номер зарегистрированного формата постоянен до конца сеанса входа:
    // регистрируем один раз. 0 — регистрация не удалась (не бывает на
    // практике); тогда секретов не видим, как до этой проверки.
    static FORMAT: OnceLock<u32> = OnceLock::new();
    let fmt = *FORMAT.get_or_init(|| {
        // SAFETY: аргумент — статическая строка UTF-16 с завершающим нулём.
        unsafe {
            RegisterClipboardFormatW(windows_sys::w!(
                "ExcludeClipboardContentFromMonitorProcessing"
            ))
        }
    });
    // SAFETY: fmt — номер формата; функция только читает состояние буфера.
    fmt != 0 && unsafe { IsClipboardFormatAvailable(fmt) } != 0
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn clipboard_holds_secret() -> bool {
    false
}

/// Windows: активное окно принадлежит процессу с повышенными правами
/// («Запуск от имени администратора»), а сам Voxy запущен без них. Ровно
/// в этом случае UIPI молча не пропускает синтетический ввод (SendInput)
/// в окно: процесс может слать ввод только окнам своего или более низкого
/// уровня целостности, а ни возвращаемое значение, ни GetLastError причину
/// не называют. Если Voxy сам запущен от администратора, UIPI не мешает,
/// и ответ — false.
///
/// Цепочка: GetForegroundWindow → GetWindowThreadProcessId →
/// OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) → OpenProcessToken
/// (TOKEN_QUERY) → GetTokenInformation(TokenElevation). Любой сбой — false
/// (тогда вставка просто пробует как обычно) и строка в логе на уровне debug.
/// Другие платформы: всегда false.
#[cfg(target_os = "windows")]
pub fn foreground_is_elevated() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    // SAFETY: функции без указателей на входе; NULL — допустимый ответ
    // (нет активного окна: переключение, экран блокировки, окно UAC).
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        log::debug!("uipi: нет активного окна");
        return false;
    }
    let mut pid: u32 = 0;
    // SAFETY: hwnd получен выше (если окно уже закрыто, функция вернёт 0),
    // pid — живая локальная переменная.
    if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 || pid == 0 {
        log::debug!(
            "uipi: GetWindowThreadProcessId не удался ({})",
            last_error()
        );
        return false;
    }
    // SAFETY: простой вызов; результат проверяется на NULL ниже.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        log::debug!("uipi: OpenProcess({pid}) не удался ({})", last_error());
        return false;
    }
    let foreground = token_elevated(process, "активное окно");
    // SAFETY: process — открытый выше дескриптор, закрывается ровно один раз.
    unsafe { CloseHandle(process) };
    if foreground != Some(true) {
        // Обычное окно (почти каждая вставка): про себя спрашивать незачем.
        return false;
    }

    // GetCurrentProcess возвращает псевдодескриптор: закрывать не нужно.
    // SAFETY: функция без аргументов.
    let own = token_elevated(unsafe { GetCurrentProcess() }, "Voxy");
    log::debug!("uipi: активное окно (pid {pid}) с повышенными правами, Voxy: {own:?}");
    uipi_blocks_input(foreground, own)
}

#[cfg(not(target_os = "windows"))]
pub fn foreground_is_elevated() -> bool {
    false
}

/// TokenElevation процесса: Some(true) — токен с повышенными правами,
/// None — узнать не удалось (причина — в логе на уровне debug).
#[cfg(target_os = "windows")]
fn token_elevated(process: windows_sys::Win32::Foundation::HANDLE, who: &str) -> Option<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::OpenProcessToken;

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: process — открытый вызывающим дескриптор (или псевдодескриптор
    // текущего процесса); token — живая локальная переменная.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        log::debug!(
            "uipi: OpenProcessToken ({who}) не удался ({})",
            last_error()
        );
        return None;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned: u32 = 0;
    // SAFETY: буфер — TOKEN_ELEVATION ровно того размера, что передан;
    // для класса TokenElevation система пишет именно эту структуру.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut std::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    let result = if ok == 0 {
        log::debug!(
            "uipi: GetTokenInformation ({who}) не удался ({})",
            last_error()
        );
        None
    } else {
        Some(elevation.TokenIsElevated != 0)
    };
    // SAFETY: token открыт выше и закрывается ровно один раз.
    unsafe { CloseHandle(token) };
    result
}

/// Код последней ошибки Win32 этого потока — для строк лога. Вызывать
/// сразу после неудачного вызова, пока его не затёр следующий.
#[cfg(target_os = "windows")]
fn last_error() -> u32 {
    // SAFETY: функция без аргументов, читает поле TEB текущего потока.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// Решение «UIPI заблокирует наш ввод»: активное окно точно с повышенными
/// правами, а Voxy — нет (неизвестное про себя считаем обычным запуском:
/// так Voxy и стартует из автозагрузки и установщика). Неизвестное про
/// окно — не блокировка: лучше обычная попытка вставки, чем ложная тревога.
#[cfg(any(target_os = "windows", test))]
fn uipi_blocks_input(foreground_elevated: Option<bool>, self_elevated: Option<bool>) -> bool {
    foreground_elevated == Some(true) && self_elevated != Some(true)
}

#[cfg(test)]
mod uipi_tests {
    use super::uipi_blocks_input;

    #[test]
    fn only_an_elevated_window_above_a_normal_voxy_blocks_input() {
        // Окно администратора, Voxy обычный (или неизвестно) — блокировка.
        assert!(uipi_blocks_input(Some(true), Some(false)));
        assert!(uipi_blocks_input(Some(true), None));
        // Voxy сам от администратора — UIPI не мешает.
        assert!(!uipi_blocks_input(Some(true), Some(true)));
        // Обычное окно или неизвестно про окно — без ложной тревоги.
        for own in [Some(true), Some(false), None] {
            assert!(!uipi_blocks_input(Some(false), own));
            assert!(!uipi_blocks_input(None, own));
        }
    }
}

#[cfg(test)]
mod sound_tests {
    use super::{sound_wav, Sound};

    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }
    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    /// PlaySoundW(SND_MEMORY) читает обычный WAV: проверяем, что встроенные
    /// файлы — канонический PCM (как пишет make-sounds.py) и короткие.
    #[test]
    fn embedded_cue_sounds_are_short_pcm_wavs() {
        for sound in [Sound::Start, Sound::Stop, Sound::Error] {
            let b = sound_wav(sound);
            assert_eq!(&b[0..4], b"RIFF");
            assert_eq!(&b[8..12], b"WAVE");
            assert_eq!(&b[12..16], b"fmt ");
            assert_eq!(u16_at(b, 20), 1, "PCM");
            assert_eq!(u16_at(b, 22), 1, "моно");
            assert_eq!(u32_at(b, 24), 44_100);
            assert_eq!(u16_at(b, 34), 16, "16 бит");
            assert_eq!(&b[36..40], b"data");
            let data = u32_at(b, 40) as usize;
            assert_eq!(b.len(), 44 + data, "размер data-чанка");
            let ms = data / 2 * 1000 / 44_100;
            assert!((50..=600).contains(&ms), "{ms} мс");
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// На отдельном буфере с уникальным именем: общий буфер обмена
    /// пользователя тест не трогает.
    #[test]
    fn concealed_and_transient_clipboard_content_is_a_secret() {
        use objc2_app_kit::NSPasteboard;
        use objc2_foundation::NSString;
        let text = NSString::from_str("public.utf8-plain-text");
        let pb = NSPasteboard::pasteboardWithUniqueName();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str("hello"), &text);
        assert!(!super::pasteboard_holds_secret(&pb));
        for marker in ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"] {
            pb.clearContents();
            pb.setString_forType(&NSString::from_str("s3cret"), &text);
            pb.setString_forType(&NSString::from_str(""), &NSString::from_str(marker));
            assert!(super::pasteboard_holds_secret(&pb), "{marker}");
        }
        // SAFETY: releaseGlobally — метод NSPasteboard без аргументов и
        // результата; буфер больше не используется.
        unsafe {
            let _: () = objc2::msg_send![&*pb, releaseGlobally];
        }
    }
}
