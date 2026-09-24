//! Платформенный шов: звуки, разрешения, клавиши вставки.
//! Для порта на Windows меняются только реализации в этом файле
//! (плюс горячая клавиша по умолчанию в store.rs).

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

#[cfg(target_os = "windows")]
pub fn play(sound: Sound) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
    let alias = match sound {
        Sound::Start => "SystemAsterisk",
        Sound::Stop => "SystemDefault",
        Sound::Error => "SystemHand",
    };
    let wide: Vec<u16> = alias.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        PlaySoundW(wide.as_ptr(), std::ptr::null_mut(), SND_ALIAS | SND_ASYNC);
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

/// Поколение буфера обмена (NSPasteboard.changeCount): растёт при каждой
/// новой записи в буфер, кто бы её ни сделал. None — платформа не сообщает
/// (Windows: пока не реализовано).
#[cfg(target_os = "macos")]
pub fn clipboard_generation() -> Option<isize> {
    // Вызывается с рабочего потока: без пула автоосвобождения временные
    // объекты AppKit на нём бы копились.
    objc2::rc::autoreleasepool(|_| {
        Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount())
    })
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
pub fn clipboard_holds_secret() -> bool {
    false
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
