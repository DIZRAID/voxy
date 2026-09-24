// Платформа и демо-режим — до стилей, чтобы не было вспышки: на macOS
// окно прозрачное (нативное стекло), на Windows и в браузере — нет.
// Отдельным файлом, а не inline-скриптом: CSP окна (tauri.conf.json)
// разрешает только скрипты из самого приложения.
(function () {
  var root = document.documentElement;
  root.dataset.platform = navigator.platform.toUpperCase().indexOf("MAC") >= 0 ? "mac" : "win";
  if (!window.__TAURI__) root.dataset.demo = "";
  root.classList.add("booting");
})();
