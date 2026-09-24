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
    let saved = clipboard.get_text().ok();

    clipboard
        .set_text(text.to_string())
        .context("не удалось записать текст в буфер")?;

    sleep(PASTE_DELAY);

    // При ошибке enigo текст намеренно остаётся в буфере обмена,
    // чтобы пользователь мог вставить его вручную.
    send_paste().context("не удалось отправить сочетание вставки")?;

    sleep(RESTORE_DELAY);
    if let Some(saved) = saved {
        let _ = clipboard.set_text(saved);
    }
    Ok(())
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
