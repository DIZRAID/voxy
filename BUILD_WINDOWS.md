# Сборка Voxy на Windows

## Что понадобится (один раз)

1. **Rust** — установите с https://rustup.rs (rustup-init.exe, все опции по умолчанию).
   Установщик сам предложит поставить **Visual Studio Build Tools** (компоненты C++) —
   согласитесь. Если пропустили: https://visualstudio.microsoft.com/visual-cpp-build-tools/ →
   галочка «Desktop development with C++».
2. **WebView2 Runtime** — на Windows 11 уже есть; на Windows 10 поставится сам при
   первом запуске приложения (или заранее: https://developer.microsoft.com/microsoft-edge/webview2/).
3. **Tauri CLI**:
   ```powershell
   cargo install tauri-cli --locked
   ```

## Перенос проекта

Скопируйте папку проекта на ПК **без** тяжёлой папки `src-tauri/target`
(это кэш сборки, он пересоздастся). Удобно через git или архив:

```powershell
# на Mac: zip без target
cd ~/Desktop && zip -r voice.zip voice -x "voice/src-tauri/target/*"
```

## Сборка и запуск

```powershell
cd voice\src-tauri
cargo tauri build
```

Готовый установщик: `src-tauri\target\release\bundle\nsis\Voxy_0.1.0_x64-setup.exe`.
Для разработки — `cargo tauri dev` (запускает приложение сразу).

## Первый запуск

- Модель (~670 МБ) скачается автоматически с Hugging Face в
  `%APPDATA%\com.dizraid.voice\models\` — прогресс виден в настройках (вкладка Model).
- **Разрешения не нужны**: в Windows нет аналога Accessibility, хоткей и вставка
  работают сразу.
- Горячая клавиша по умолчанию — **правый Ctrl** (меняется в настройках; настройки
  и история переносимы между macOS и Windows — файл settings.json совместим).
- SmartScreen при запуске установщика может предупредить о неизвестном издателе
  (приложение не подписано сертификатом) — «Подробнее» → «Выполнить в любом случае».

## Отличия от macOS-версии

- Островок в покое скрыт (нет выреза камеры) и выезжает сверху при записи.
- Вставка — синтетический **Ctrl+V** (виртуальный код, работает на любой раскладке).
- Звуки — системные алиасы Windows.
- Лог: `%LOCALAPPDATA%\Voxy.log`.

## Ускорение на NVIDIA GPU (опционально, потом)

Сейчас распознавание идёт на CPU (быстро: ~15–30× реального времени + стриминг).
Если захочется CUDA — понадобится собрать sherpa-onnx с CUDA-провайдером и
переключить `provider` в `asr.rs`; вернёмся к этому после стабильной CPU-версии.
