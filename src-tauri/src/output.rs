//! Вставка распознанного текста в позицию курсора активного приложения:
//! сохранить буфер обмена → записать текст → синтетический Cmd+V → восстановить.

use anyhow::{Context, Result};
use enigo::{Direction, Enigo, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

/// Пауза после записи в буфер и физического отпускания хоткея,
/// прежде чем слать синтетический Cmd+V.
const PASTE_DELAY: Duration = Duration::from_millis(150);
/// Сколько ждать, пока целевое приложение прочитает буфер, до восстановления.
const RESTORE_DELAY: Duration = Duration::from_millis(500);

/// Вставка не отправлена: активное окно запущено от администратора, а Voxy
/// — нет, и Windows (UIPI) не пропустит синтетический Ctrl+V (см.
/// platform::foreground_is_elevated). Текст остаётся в буфере обмена.
/// insert_text возвращает её как anyhow::Error; worker узнаёт её через
/// downcast_ref и показывает на островке отдельное сообщение.
#[derive(Debug)]
pub struct PasteBlocked;

impl std::fmt::Display for PasteBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the active window runs elevated; Windows (UIPI) blocks synthetic input")
    }
}

impl std::error::Error for PasteBlocked {}

pub fn insert_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("нет доступа к буферу обмена")?;

    // Не-текстовое содержимое (картинка) вернёт Err — тогда не восстанавливаем.
    // Секрет из менеджера паролей тоже не восстанавливаем (см.
    // platform::clipboard_holds_secret): он пропадёт из буфера, как после
    // обычной автоочистки менеджера.
    let saved = if crate::platform::clipboard_holds_secret() {
        None
    } else {
        clipboard.get_text().ok()
    };

    set_text_private(&mut clipboard, text).context("не удалось записать текст в буфер")?;
    let generation = crate::platform::clipboard_generation();

    sleep(PASTE_DELAY);

    // Окно администратора (Windows, UIPI): Ctrl+V до него не дойдёт, а
    // SendInput может об этом и не сообщить (документация обещает лишь,
    // что причину не назовут) — тогда буфер восстановился бы поверх
    // непрочитанного текста. Поэтому проверяем до отправки и сразу выходим,
    // оставив текст в буфере для ручной вставки. На macOS всегда false.
    // Проверка — несколько системных вызовов, микросекунды.
    if crate::platform::foreground_is_elevated() {
        return Err(anyhow::Error::new(PasteBlocked));
    }

    // При ошибке enigo текст намеренно остаётся в буфере обмена,
    // чтобы пользователь мог вставить его вручную.
    send_paste().context("не удалось отправить сочетание вставки")?;

    sleep(RESTORE_DELAY);
    if let Some(saved) = saved {
        // Пока шла вставка, в буфер записал кто-то другой (пользователь
        // что-то скопировал) — его содержимое не затираем.
        if crate::platform::clipboard_generation() == generation {
            let _ = set_text_private(&mut clipboard, &saved);
        } else {
            log::info!("буфер обмена изменился во время вставки, не восстанавливаю");
        }
    }
    Ok(())
}

/// Запись в буфер с пометкой «не для истории буфера обмена»: ни
/// продиктованный текст, ни возвращённое содержимое не должны появляться
/// в Maccy/Raycast/Paste (macOS) или в журнале Win+V как новые записи.
fn set_text_private(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    #[cfg(target_os = "macos")]
    {
        use arboard::SetExtApple;
        clipboard.set().exclude_from_history().text(text)
    }
    #[cfg(target_os = "windows")]
    {
        use arboard::SetExtWindows;
        clipboard.set().exclude_from_history().text(text)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        clipboard.set_text(text)
    }
}

fn send_paste() -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())
        .context("enigo: could not send the paste shortcut (macOS: Accessibility permission?)")?;
    let modifier = crate::platform::paste_modifier();
    let v = crate::platform::v_key();

    enigo.key(modifier, Direction::Press)?;
    enigo.key(v, Direction::Click)?;
    sleep(Duration::from_millis(100));
    enigo.key(modifier, Direction::Release)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::PasteBlocked;

    /// Контракт с worker.rs: ошибка insert_text узнаётся через downcast_ref,
    /// даже если по дороге в worker к ней добавят контекст.
    #[test]
    fn paste_blocked_survives_the_anyhow_round_trip() {
        let e = anyhow::Error::new(PasteBlocked);
        assert!(e.downcast_ref::<PasteBlocked>().is_some());
        assert!(e.to_string().contains("UIPI"));
        let wrapped = e.context("вставка");
        assert!(wrapped.downcast_ref::<PasteBlocked>().is_some());

        let other = anyhow::anyhow!("enigo: SendInput failed");
        assert!(other.downcast_ref::<PasteBlocked>().is_none());
    }
}
