// Страница — окно приложения, а не вкладка браузера. WebView2 (Windows)
// оставляет браузерные сочетания и контекстное меню Chromium (Back, Refresh,
// Print, Inspect), пока страница их не отменит: F5 перезагрузил бы
// настройки (в том числе когда хоткей — F5 и окно в фокусе), Ctrl+P открыл
// бы печать, Ctrl+F — поиск. WKWebView (macOS) показывает своё меню с
// Reload. Подключается обеими страницами (settings.html, island.html).
//
// Захват хоткея идёт в Rust через глобальный хук/тап, поэтому
// preventDefault здесь ни захвату, ни самому хоткею не мешает; событие не
// останавливается, обработчики страницы (Esc, стрелки, демо-захват) его
// получают как прежде.
(function () {
  "use strict";
  var F_KEYS = { F3: 1, F5: 1, F7: 1, F12: 1 }; // поиск, обновить, каретка, DevTools
  var BROWSER_KEYS = { BrowserBack: 1, BrowserForward: 1, BrowserRefresh: 1, BrowserSearch: 1 };
  // Ctrl/Cmd + буква: и по символу, и по физической клавише (KeyR и т.д.),
  // чтобы ловить сочетания и на кириллической раскладке.
  var MOD_KEYS = { r: 1, p: 1, f: 1, u: 1, g: 1 }; // обновить, печать, поиск, исходник, следующее
  var MOD_SHIFT_KEYS = { i: 1, j: 1, c: 1 }; // DevTools, консоль, инспектор

  function letter(e) {
    if (/^Key[A-Z]$/.test(e.code || "")) return e.code.charAt(3).toLowerCase();
    return "";
  }

  document.addEventListener(
    "keydown",
    function (e) {
      if (F_KEYS[e.key] || BROWSER_KEYS[e.key]) {
        e.preventDefault();
        return;
      }
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      var k = (e.key || "").toLowerCase();
      var c = letter(e);
      if (MOD_KEYS[k] || MOD_KEYS[c] || (e.shiftKey && (MOD_SHIFT_KEYS[k] || MOD_SHIFT_KEYS[c]))) {
        e.preventDefault();
      }
    },
    true
  );

  // Меню остаётся в полях ввода (вставить ключ API) и на выделенном тексте
  // (копировать из истории или текста ошибки).
  document.addEventListener(
    "contextmenu",
    function (e) {
      var t = e.target;
      if (t && t.closest && t.closest("input, textarea")) return;
      var sel = window.getSelection ? window.getSelection() : null;
      if (sel && !sel.isCollapsed && String(sel).trim() && t && sel.containsNode(t, true)) return;
      e.preventDefault();
    },
    true
  );
})();
