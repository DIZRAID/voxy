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
        .context("enigo: нет разрешения Accessibility?")?;
    let modifier = crate::platform::paste_modifier();
    let v = crate::platform::v_key();

    enigo.key(modifier, Direction::Press)?;
    enigo.key(v, Direction::Click)?;
    sleep(Duration::from_millis(100));
    enigo.key(modifier, Direction::Release)?;
    Ok(())
}
