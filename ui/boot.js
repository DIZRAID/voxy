// Платформа, подложка окна и демо-режим — до стилей, чтобы не было вспышки.
// Отдельным файлом, а не inline-скриптом: CSP окна (tauri.conf.json)
// разрешает только скрипты из самого приложения.
//
// Единственный источник платформы для settings.css и settings.js
// (settings.js читает <html data-platform>):
// - в Tauri — navigator.platform: "MacIntel" на macOS, "Win32" в WebView2;
// - в браузерном превью (без Tauri) ?platform=win|mac выбирает раскладку;
//   без параметра превью на Windows показывает Windows, в остальных
//   браузерах (macOS, Linux) — macOS, как и раньше.
// data-backdrop (контракт C1): Rust открывает окно на Windows как
// settings.html?backdrop=mica (Windows 11, под окном Mica) или
// ?backdrop=opaque (Windows 10, непрозрачное окно). На macOS параметра нет.
// В превью Windows по умолчанию opaque. Неизвестное значение игнорируется:
// без атрибута CSS рисует непрозрачный вариант (безопасно на любой сборке).
(function () {
  var root = document.documentElement;
  var tauri = !!window.__TAURI__;
  var q = new URLSearchParams(location.search);
  var platform;
  if (tauri) {
    platform = navigator.platform.toUpperCase().indexOf("MAC") >= 0 ? "mac" : "win";
  } else {
    platform = q.get("platform");
    if (platform !== "win" && platform !== "mac") {
      platform = /^WIN/i.test(navigator.platform) ? "win" : "mac";
    }
  }
  root.dataset.platform = platform;
  var backdrop = q.get("backdrop");
  if (backdrop !== "mica" && backdrop !== "opaque") backdrop = null;
  if (!backdrop && !tauri && platform === "win") backdrop = "opaque";
  if (backdrop) root.dataset.backdrop = backdrop;
  if (!tauri) root.dataset.demo = "";
  root.classList.add("booting");
})();
