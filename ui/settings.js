// Voxy — окно настроек. Классический скрипт без модулей и сборки: страница
// открывается и из file:// / простого http-сервера (демо-превью без Tauri).
// Окно уничтожается при закрытии, поэтому при каждом открытии всё состояние
// заново берётся из Rust (get_settings, get_model_status, permissions_status).
"use strict";

// ================================================================= bridge
const IS_TAURI = !!window.__TAURI__;
const IS_MAC = navigator.platform.toUpperCase().includes("MAC");
// Вне Tauri — демо с моками и имитацией событий (см. createDemo в конце).
const DEMO = IS_TAURI ? null : createDemo();
const invoke = IS_TAURI ? window.__TAURI__.core.invoke : DEMO.invoke;
const listen = IS_TAURI ? window.__TAURI__.event.listen : DEMO.listen;

const logErr = (e) => console.error("[settings]", e);

function $(id) {
  return document.getElementById(id);
}

// Мини-hyperscript: атрибуты строками, дети — узлы или текст (без innerHTML).
function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") el.className = v;
      else el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid);
  return el;
}

const ICON = {
  check: '<path d="M4.5 12.5l5 5 10-11"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>',
  cloud: '<path d="M7 18a4.5 4.5 0 0 1-.6-8.96A6 6 0 0 1 18 9.5a3.8 3.8 0 0 1-.5 7.5M12 12v7M9.5 16.5L12 19l2.5-2.5"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="M10.8 12.2L19 4M16 7l2 2M14 9l2 2"/>',
  sparkle: '<path d="M11 3.5l1.8 5.2 5.2 1.8-5.2 1.8L11 17.5l-1.8-5.2L4 10.5l5.2-1.8z"/><path d="M19 15.5v5M16.5 18h5"/>',
};
const iconCache = new Map();
function icon(markup, cls) {
  const key = `${cls || ""}|${markup}`;
  let proto = iconCache.get(key);
  if (!proto) {
    const t = document.createElement("template");
    t.innerHTML = `<svg viewBox="0 0 24 24"${cls ? ` class="${cls}"` : ""} aria-hidden="true">${markup}</svg>`;
    proto = t.content.firstElementChild;
    iconCache.set(key, proto);
  }
  return proto.cloneNode(true);
}

// ================================================================== state
let settings = null; // объект из get_settings; меняется на месте и уходит целиком
let mics = [];
let overview = null; // последний models_overview
let activeId = null; // активный движок (id модели или "online:<provider>")
const engine = { status: null, message: null, labelFor: null, name: "", vendor: "" };
let statusSeq = 0; // растёт на каждое событие model-status
const rowErrors = {}; // id → текст ошибки под строкой
const keyForms = {}; // provider → { value, checking, enter } — открытые формы ключа
const downloads = {}; // id → { progress, state, done, total }
const busy = new Set(); // id строк, у которых идёт действие
let capturing = false;
let currentTab = "general";
let lastPerm = null; // последний permissions_status / событие permissions

// Без обоих разрешений (macOS) тапа в Rust нет: захват хоткея не закончился бы.
function permsMissing() {
  return !!lastPerm && !(lastPerm.accessibility && lastPerm.input_monitoring);
}

// Всегда весь объект: у Settings в Rust #[serde(default)], пропущенное поле
// сбросилось бы в значение по умолчанию.
function saveSettings() {
  if (!settings) return Promise.resolve();
  return invoke("set_settings", { newSettings: settings }).catch(logErr);
}

// ============================================================== key names
const KEY_NAMES = IS_MAC
  ? {
      MetaRight: "⌘ Right Command",
      MetaLeft: "⌘ Left Command",
      ControlRight: "⌃ Right Control",
      ControlLeft: "⌃ Left Control",
      Alt: "⌥ Left Option",
      AltGr: "⌥ Right Option",
      ShiftRight: "⇧ Right Shift",
      ShiftLeft: "⇧ Left Shift",
      CapsLock: "⇪ Caps Lock",
      Function: "Fn",
    }
  : {
      MetaRight: "Right Win",
      MetaLeft: "Left Win",
      ControlRight: "Right Ctrl",
      ControlLeft: "Left Ctrl",
      Alt: "Left Alt",
      AltGr: "Right Alt",
      ShiftRight: "Right Shift",
      ShiftLeft: "Left Shift",
      CapsLock: "Caps Lock",
      Function: "Fn",
    };
const DEFAULT_HOTKEY = IS_MAC ? "MetaRight" : "ControlRight";

function keyDisplay(name) {
  if (KEY_NAMES[name]) return KEY_NAMES[name];
  if (/^F\d+$/.test(name)) return name;
  const m = /^Key0x([0-9A-F]+)$/i.exec(name);
  if (m) return `Key 0x${m[1]}`;
  return name;
}

// «⌘ Right Command» → колпачок «⌘» + «Right Command»; без символа — «⌨».
function splitKey(name) {
  const text = keyDisplay(name);
  const m = /^([⌘⌃⌥⇧⇪])\s*(.*)$/.exec(text);
  return m ? { cap: m[1], label: m[2] } : { cap: "⌨", label: text };
}

// ================================================================== menus
// Меню по макету: стекло, пункты 26 px, колонка галочки. Одно открытое
// меню за раз; закрывается кликом снаружи, выбором, Esc и потерей фокуса окна.
let openMenu = null;
let menuUid = 0;

function closeMenu(restoreFocus) {
  if (openMenu) openMenu.close(restoreFocus);
}

function createMenu({ anchor, trigger, width, head, up, label, getItems, onPick }) {
  const uid = `menu-${++menuUid}`;
  let menu = null;
  let items = [];
  let els = [];
  let cur = -1;
  trigger.setAttribute("aria-haspopup", "menu");
  trigger.setAttribute("aria-expanded", "false");

  function setCur(i) {
    cur = i;
    els.forEach((el, j) => el.classList.toggle("cur", j === i));
    if (!menu) return;
    if (i >= 0) menu.setAttribute("aria-activedescendant", els[i].id);
    else menu.removeAttribute("aria-activedescendant");
  }
  function onOutside(e) {
    if (menu && !menu.contains(e.target) && !trigger.contains(e.target)) close(false);
  }
  function close(restoreFocus) {
    if (!menu) return;
    // Фокус был в меню (например, окно потеряло фокус) — он вернётся на
    // кнопку, а не в body.
    const hadFocus = menu.contains(document.activeElement);
    menu.remove();
    menu = null;
    els = [];
    cur = -1;
    openMenu = null;
    trigger.setAttribute("aria-expanded", "false");
    trigger.removeAttribute("aria-controls");
    document.removeEventListener("pointerdown", onOutside, true);
    if (restoreFocus || hadFocus) trigger.focus({ preventScroll: true });
  }
  function pick(i) {
    const item = items[i];
    close(true);
    onPick(item);
  }
  function onKey(e) {
    const n = els.length;
    switch (e.key) {
      case "ArrowDown":
        setCur((cur + 1) % n);
        break;
      case "ArrowUp":
        setCur(cur <= 0 ? n - 1 : cur - 1);
        break;
      case "Home":
        setCur(0);
        break;
      case "End":
        setCur(n - 1);
        break;
      case "Enter":
      case " ":
        if (cur >= 0) pick(cur);
        break;
      case "Escape":
        close(true);
        break;
      case "Tab":
        close(true); // фокус на кнопку, дальше Tab уходит как обычно
        return;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }
  function open(viaKeyboard) {
    closeMenu(false);
    items = getItems();
    menu = h("div", {
      class: `menu${up ? " up" : ""}`,
      role: "menu",
      id: uid,
      tabindex: "-1",
      "aria-label": label,
      style: `width:${width}px`,
    });
    if (head) menu.append(h("div", { class: "menu-head", role: "presentation" }, head));
    els = items.map((it, i) => {
      const el = h(
        "div",
        { class: "menu-item", role: "menuitemradio", id: `${uid}-${i}`, "aria-checked": String(!!it.on), title: it.title },
        h("span", { class: "menu-check", "aria-hidden": "true" }, it.on ? icon(ICON.check) : null),
        h("span", { class: "menu-label" }, it.name)
      );
      el.addEventListener("click", () => pick(i));
      el.addEventListener("pointermove", () => {
        if (cur !== i) setCur(i);
      });
      return el;
    });
    menu.append(...els);
    menu.addEventListener("pointerleave", () => setCur(-1));
    menu.addEventListener("keydown", onKey);
    anchor.append(menu);
    // Вверх — только если над кнопкой хватает места до конца маски заголовка
    // (62 px от верха .content); иначе меню ушло бы под неё и за край прокрутки.
    if (up) {
      const room = trigger.getBoundingClientRect().top - $("content").getBoundingClientRect().top - 62;
      if (room < menu.offsetHeight + 6) menu.classList.remove("up");
    }
    trigger.setAttribute("aria-expanded", "true");
    trigger.setAttribute("aria-controls", uid);
    document.addEventListener("pointerdown", onOutside, true);
    openMenu = { close };
    menu.focus({ preventScroll: true });
    if (viaKeyboard) setCur(Math.max(0, items.findIndex((it) => it.on)));
  }

  trigger.addEventListener("click", (e) => {
    if (!settings) return;
    if (menu) close(true);
    else open(e.detail === 0); // detail 0 — нажатие с клавиатуры
  });
  trigger.addEventListener("keydown", (e) => {
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !menu && settings) {
      e.preventDefault();
      open(true);
    }
  });
  return { close };
}

// ================================================================ general
function renderHotkey() {
  $("hk-chip").classList.toggle("capturing", capturing);
  if (capturing) {
    $("hk-cap").textContent = "…";
    $("hk-name").textContent = "Press keys…";
  } else {
    const k = splitKey(settings ? settings.hotkey : DEFAULT_HOTKEY);
    $("hk-cap").textContent = k.cap;
    $("hk-name").textContent = k.label;
  }
  const btn = $("hk-btn");
  btn.textContent = capturing ? "Cancel" : "Change";
  const blocked = !capturing && permsMissing();
  if (blocked) {
    btn.setAttribute("aria-disabled", "true");
    btn.title = "Grant the permissions above first";
  } else {
    btn.removeAttribute("aria-disabled");
    btn.removeAttribute("title");
  }
  renderEmptyHint();
}

// Захват идёт через глобальный тап в Rust: следующая нажатая клавиша
// сохраняется там же и приходит событием hotkey-captured.
function beginCapture() {
  capturing = true;
  renderHotkey();
  invoke("begin_hotkey_capture").catch(logErr);
}
function cancelCapture() {
  capturing = false;
  renderHotkey();
  invoke("cancel_hotkey_capture").catch(logErr);
}
$("hk-btn").addEventListener("click", () => {
  if (!settings) return;
  if (capturing) return cancelCapture();
  if (permsMissing()) {
    // захват не начнётся: ведём к кнопке недостающего разрешения
    const pill = document.querySelector(".perm-actions .pill:not(.hidden)");
    if (pill) pill.focus();
    return;
  }
  beginCapture();
});

listen("hotkey-captured", (e) => {
  const p = e.payload || {};
  capturing = false;
  if (!p.cancelled && p.key && settings) settings.hotkey = p.key;
  renderHotkey();
});

createMenu({
  anchor: $("mic-anchor"),
  trigger: $("mic-trigger"),
  width: 220,
  label: "Microphone",
  getItems: () => [
    { name: "System default", value: null, on: !settings.mic_device },
    ...mics.map((m) => ({ name: m, value: m, title: m, on: settings.mic_device === m })),
  ],
  onPick: (item) => {
    settings.mic_device = item.value;
    renderMicLabel();
    saveSettings();
  },
});
function renderMicLabel() {
  const name = (settings && settings.mic_device) || "System default";
  $("mic-label").textContent = name;
  $("mic-label").title = name;
}

function bindSwitch(id, key) {
  const el = $(id);
  el.addEventListener("click", () => {
    if (!settings) return;
    settings[key] = !settings[key];
    el.setAttribute("aria-checked", String(!!settings[key]));
    saveSettings();
  });
}
bindSwitch("sounds-toggle", "sounds");
bindSwitch("autostart-toggle", "autostart");

// ============================================================== recording
const MODES = [
  { id: "ptt", hint: "Hold the key while you speak" },
  { id: "toggle", hint: "Tap to start, tap again to stop" },
  { id: "dynamic", hint: "Tap to toggle, hold ≥1 s to talk" },
];
const segButtons = [...document.querySelectorAll("#mode-seg button")];

function renderMode() {
  const mode = settings ? settings.mode : "ptt";
  const idx = Math.max(0, MODES.findIndex((m) => m.id === mode));
  $("seg-thumb").style.transform = `translateX(${idx * 72}px)`;
  segButtons.forEach((b, i) => {
    b.setAttribute("aria-checked", String(b.dataset.mode === mode));
    b.tabIndex = i === idx ? 0 : -1;
  });
  $("mode-hint").textContent = MODES[idx].hint;
  renderEmptyHint();
}
function setMode(id) {
  if (!settings) return;
  settings.mode = id;
  renderMode();
  saveSettings();
}
segButtons.forEach((b) => b.addEventListener("click", () => setMode(b.dataset.mode)));
$("mode-seg").addEventListener("keydown", (e) => {
  const i = segButtons.indexOf(document.activeElement);
  if (i < 0) return;
  let n = null;
  if (e.key === "ArrowRight" || e.key === "ArrowDown") n = Math.min(segButtons.length - 1, i + 1);
  else if (e.key === "ArrowLeft" || e.key === "ArrowUp") n = Math.max(0, i - 1);
  if (n === null || n === i) return;
  e.preventDefault();
  setMode(segButtons[n].dataset.mode);
  segButtons[n].focus();
});

// Ползунок: 1:1 за пальцем с сохранением точки захвата на ручке, «резинка»
// за краями и пружина обратно. Значение — целое на сетке шага; сохраняется
// при отпускании (как раньше), с клавиатуры — по keyup.
function createSlider(el, valueEl, fmt, get, set) {
  const min = +el.dataset.min;
  const max = +el.dataset.max;
  const step = +el.dataset.step;
  const knob = el.querySelector(".slider-knob");
  const fill = el.querySelector(".slider-fill");
  const snap = (v) => Math.min(max, Math.max(min, Math.round(v / step) * step));
  let drag = null;
  let keyDirty = false;

  function render(rb = 0, scale = 1) {
    if (!settings) return;
    const v = get();
    const pct = ((v - min) / (max - min)) * 100;
    fill.style.width = `${pct}%`;
    knob.style.left = `${pct}%`;
    knob.style.transform = rb || scale !== 1 ? `translateX(${rb.toFixed(1)}px) scale(${scale})` : "";
    valueEl.textContent = fmt(v);
    el.setAttribute("aria-valuenow", String(v));
    el.setAttribute("aria-valuetext", fmt(v));
  }
  function move(x) {
    const { rect, grab } = drag;
    const px = x - grab - rect.left;
    const t = Math.min(1, Math.max(0, px / rect.width));
    const over = px < 0 ? px : px > rect.width ? px - rect.width : 0;
    const rb = over ? (over * 24 * 0.55) / (24 + 0.55 * Math.abs(over)) : 0;
    set(snap(min + t * (max - min)));
    render(rb, 1.15);
  }
  function end(e) {
    if (!drag || e.pointerId !== drag.id) return;
    drag = null;
    el.classList.remove("dragging");
    render();
    saveSettings();
  }
  el.addEventListener("pointerdown", (e) => {
    if (!settings || e.button !== 0) return;
    const rect = el.getBoundingClientRect();
    const knobX = rect.left + ((get() - min) / (max - min)) * rect.width;
    const grab = Math.abs(e.clientX - knobX) <= 14 ? e.clientX - knobX : 0;
    drag = { rect, grab, id: e.pointerId };
    try {
      el.setPointerCapture(e.pointerId);
    } catch (_) {
      /* указатель уже отпущен */
    }
    el.classList.add("dragging");
    move(e.clientX);
  });
  el.addEventListener("pointermove", (e) => {
    if (drag && e.pointerId === drag.id) move(e.clientX);
  });
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
  el.addEventListener("lostpointercapture", end);

  el.addEventListener("keydown", (e) => {
    if (!settings) return;
    const v = get();
    let n;
    switch (e.key) {
      case "ArrowLeft":
      case "ArrowDown":
        n = v - step;
        break;
      case "ArrowRight":
      case "ArrowUp":
        n = v + step;
        break;
      case "PageDown":
        n = v - step * 4;
        break;
      case "PageUp":
        n = v + step * 4;
        break;
      case "Home":
        n = min;
        break;
      case "End":
        n = max;
        break;
      default:
        return;
    }
    e.preventDefault();
    n = snap(n);
    if (n === v) return;
    set(n);
    render();
    keyDirty = true;
  });
  const flush = () => {
    if (!keyDirty) return;
    keyDirty = false;
    saveSettings();
  };
  el.addEventListener("keyup", flush);
  el.addEventListener("blur", flush);
  return { render };
}

const minSlider = createSlider(
  $("minlen-slider"),
  $("minlen-val"),
  (v) => `${v} ms`,
  () => settings.min_duration_ms,
  (v) => (settings.min_duration_ms = v)
);
const maxSlider = createSlider(
  $("maxlen-slider"),
  $("maxlen-val"),
  (v) => `${v} s`,
  () => settings.max_record_s,
  (v) => (settings.max_record_s = v)
);

// ================================================================= models
const LABELS = {
  best_balance: "Best balance",
  most_languages: "Most languages",
  asian_languages: "Asian languages",
  fastest: "Fastest",
  smallest: "Smallest",
};
const FAMILY_NAMES = {
  nemo_transducer: "Parakeet",
  whisper: "Whisper",
  moonshine: "Moonshine",
  qwen3_asr: "Qwen3-ASR",
};
const UNLOAD_OPTIONS = [
  { value: 0, name: "Never" },
  { value: 2, name: "After 2 min" },
  { value: 5, name: "After 5 min" },
  { value: 15, name: "After 15 min" },
  { value: 30, name: "After 30 min" },
];
const TERMINAL = new Set(["done", "cancelled", "error"]);
const RING = 69.12; // длина окружности r=11

const fmtSize = (mb) => (mb >= 1000 ? `${(mb / 1000).toFixed(2)} GB` : `${Math.round(mb)} MB`);
const fmtRam = (mb) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);
const dash = (p) => `${((Math.max(0, Math.min(100, p || 0)) / 100) * RING).toFixed(1)} ${RING}`;

function langShort(m) {
  if (m.english_only) return "English";
  if (m.languages > 1) return `${m.languages} languages`;
  return m.languages_note;
}

function fmtReleased(s) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s || "");
  if (!m) return s;
  return new Date(+m[1], +m[2] - 1, +m[3]).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function downloadOf(m) {
  return downloads[m.id] || (m.downloading ? { progress: 0, state: "downloading" } : null);
}
function isDownloadingId(id) {
  if (downloads[id]) return true;
  return !!(overview && overview.models.some((m) => m.id === id && m.downloading));
}

function dlMeta(d) {
  if (d.state === "verifying") return "Checking…";
  if (d.state === "extracting") return "Unpacking…";
  if (d.total > 0 && d.done != null) return `Downloading · ${fmtSize(d.done / 1e6)} of ${fmtSize(d.total / 1e6)}`;
  return "Downloading…";
}

// Общая обёртка действий строки: ошибка остаётся под строкой до следующего
// действия с ней; после — всегда свежий models_overview. Повторный клик,
// пока действие идёт, игнорируется.
async function act(id, fn) {
  if (busy.has(id)) return;
  busy.add(id);
  delete rowErrors[id];
  try {
    await fn();
  } catch (e) {
    rowErrors[id] = String(e);
  } finally {
    busy.delete(id);
  }
  await loadModels();
}

let loadSeq = 0;
async function loadModels() {
  const my = ++loadSeq;
  const statusAt = statusSeq;
  let ov;
  try {
    ov = await invoke("models_overview");
  } catch (e) {
    logErr(e);
    // без обзора страница не остаётся пустой: строка выгрузки видна (CSS)
    if (!overview) $("tab-model").classList.add("no-overview");
    return;
  }
  // Ответы применяются по порядку запросов: устаревший не затирает новый.
  if (my !== loadSeq || !ov) return;
  overview = ov;
  activeId = ov.active;
  // Живое событие model-status свежее статуса из обзора.
  if (statusAt === statusSeq) engine.status = ov.status;
  for (const m of ov.models) {
    if (!m.downloading) delete downloads[m.id];
  }
  renderModels();
  renderStatusCard();
}

// Перерисовка внутри scope с сохранением фокуса и выделения в поле. После
// render() ищется тот же data-fk, иначе элемент той же строки (строка могла
// переехать в другой список: удалили, докачали), иначе выбранное радио.
// Один вызов на всю перерисовку: вложенный искал бы только в своём списке.
function keepFocus(scope, render) {
  const a = document.activeElement;
  const inside = !!a && a !== document.body && scope.contains(a);
  const fk = inside ? a.dataset.fk : null;
  const rowKey = inside ? a.closest("[data-row]")?.dataset.row : null;
  const sel = inside && a.tagName === "INPUT" ? [a.selectionStart, a.selectionEnd] : null;
  render();
  if (!inside || scope.contains(document.activeElement)) return;
  const q = (s) => scope.querySelector(s);
  const target =
    (fk && q(`[data-fk="${CSS.escape(fk)}"]`)) ||
    (rowKey && q(`[data-row="${CSS.escape(rowKey)}"] [data-fk]`)) ||
    q('[role="radio"][tabindex="0"]');
  if (!target) return;
  target.focus({ preventScroll: true });
  if (sel && target.setSelectionRange) {
    try {
      target.setSelectionRange(sel[0], sel[1]);
    } catch (_) {
      /* type=password без выделения */
    }
  }
}

// Радио-строки: Tab попадает на выбранную, стрелки двигают фокус (выбор —
// Enter/Space или клик, чтобы не перезагружать модели на каждой стрелке).
function fixRoving(container) {
  const radios = [...container.querySelectorAll('[role="radio"]')];
  const cur = radios.find((r) => r.dataset.on === "1") || radios[0];
  radios.forEach((r) => (r.tabIndex = r === cur ? 0 : -1));
}
function setupRoving(container) {
  container.addEventListener("keydown", (e) => {
    const radios = [...container.querySelectorAll('[role="radio"]')];
    const i = radios.indexOf(document.activeElement);
    if (i < 0) return;
    let n = null;
    if (e.key === "ArrowDown" || e.key === "ArrowRight") n = (i + 1) % radios.length;
    else if (e.key === "ArrowUp" || e.key === "ArrowLeft") n = (i - 1 + radios.length) % radios.length;
    else if (e.key === "Home") n = 0;
    else if (e.key === "End") n = radios.length - 1;
    if (n === null) return;
    e.preventDefault();
    radios.forEach((r, j) => (r.tabIndex = j === n ? 0 : -1));
    radios[n].focus();
  });
}
setupRoving($("installed-list"));
setupRoving($("online-list"));

function activateOn(row, main, fn) {
  row.addEventListener("click", (e) => {
    // текст ошибки выделяют и копируют — это не выбор модели
    if (!e.target.closest("button, input, form, .m-err")) fn();
  });
  main.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fn();
    }
  });
}

// Кнопка действия в строке. Действие сразу перерисовывает список и под
// курсор встаёт другая кнопка (корзина следующей модели, «стоп» вместо Get),
// поэтому второй клик двойного игнорируется: detail — счётчик кликов ОС,
// с клавиатуры он 0.
function onAction(btn, fn) {
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (e.detail > 1) return;
    fn();
  });
  return btn;
}

function trashButton(label, title, fk, onClick) {
  return onAction(
    h("button", { type: "button", class: "icon-btn", "aria-label": label, title, "data-fk": fk }, icon(ICON.trash)),
    onClick
  );
}

function errLine(row, id) {
  const msg = rowErrors[id];
  if (msg) row.append(h("div", { class: "m-err", title: msg }, msg));
}

function chipsFor(m) {
  const chips = (m.labels || []).map((l) => h("span", { class: "chip" }, LABELS[l] || l));
  if (m.speed === "slow") chips.push(h("span", { class: "chip plain" }, "Slower"));
  return chips;
}
function nameLine(m) {
  return h("div", { class: "m-name" }, h("span", { class: "m-title", id: `name-${m.id}` }, m.name), chipsFor(m));
}

// Радио выбранной строки анимируется: строка строится в прежнем состоянии
// и переключается после вставки в DOM (переходы из макета).
let shownActive; // активный id на момент прошлой отрисовки
function withFlip(flips, row, main, on) {
  const was = shownActive === undefined ? on : shownActive === row.dataset.row;
  row.classList.toggle("is-active", was);
  main.setAttribute("aria-checked", String(was));
  main.dataset.on = on ? "1" : "0";
  if (was !== on) {
    flips.push(() => {
      row.classList.toggle("is-active", on);
      main.setAttribute("aria-checked", String(on));
    });
  }
}
function runFlips(container, flips) {
  if (!flips.length) return;
  void container.offsetWidth;
  flips.forEach((f) => f());
}

function installedRow(m, flips) {
  const on = overview.active === m.id;
  const main = h(
    "div",
    {
      class: "mrow-main",
      role: "radio",
      tabindex: "-1",
      "data-fk": `use:${m.id}`,
      "aria-labelledby": `name-${m.id}`,
      "aria-describedby": `meta-${m.id}`,
    },
    h("span", { class: "radio", "aria-hidden": "true" }),
    h(
      "div",
      null,
      nameLine(m),
      h(
        "div",
        { class: "m-meta", id: `meta-${m.id}`, title: m.languages_note },
        `${m.vendor} · ${langShort(m)} · ${fmtSize(m.size_mb)} · ${fmtRam(m.ram_mb)} RAM`
      )
    )
  );
  const actions = h("div", { class: "m-actions" });
  if (on) {
    actions.append(h("span", { class: "in-use" }, "In use"));
  } else {
    actions.append(
      trashButton(`Delete ${m.name}`, IS_MAC ? "Delete from this Mac" : "Delete from this computer", `del:${m.id}`, () =>
        act(m.id, () => invoke("model_delete", { id: m.id }))
      )
    );
  }
  const row = h("div", { class: `mrow hoverable${on ? "" : " selectable"}`, "data-row": m.id }, main, actions);
  withFlip(flips, row, main, on);
  errLine(row, m.id);
  if (!on) activateOn(row, main, () => act(m.id, () => invoke("model_activate", { id: m.id })));
  return row;
}

function ringButton(m, progress) {
  const b = h("button", {
    type: "button",
    class: "ring-btn",
    title: "Stop download",
    "aria-label": `Stop downloading ${m.name}`,
    "data-fk": `stop:${m.id}`,
  });
  // статическая разметка, данных внутри нет
  b.innerHTML =
    '<svg viewBox="0 0 26 26" aria-hidden="true"><circle class="ring-track" cx="13" cy="13" r="11"/><circle class="ring-fill" cx="13" cy="13" r="11"/></svg><span class="ring-stop"></span>';
  b.querySelector(".ring-fill").style.strokeDasharray = dash(progress);
  return onAction(b, () => act(m.id, () => invoke("model_cancel_download", { id: m.id })));
}

function startDownload(m) {
  if (busy.has(m.id)) return;
  // оптимистично: кольцо сразу; правду скажет overview.downloading
  downloads[m.id] = { progress: 0, state: "downloading" };
  renderModels();
  renderStatusCard();
  act(m.id, () => invoke("model_download", { id: m.id }));
}

function availableRow(m) {
  const d = downloadOf(m);
  const meta = h(
    "div",
    { class: "m-meta", title: m.languages_note },
    d ? dlMeta(d) : `${m.vendor} · ${langShort(m)} · ${fmtSize(m.download_mb)}`
  );
  const main = h("div", { class: "mrow-main" }, icon(ICON.cloud, "lead-icon"), h("div", null, nameLine(m), meta));
  const actions = h("div", { class: "m-actions dl" });
  const row = h("div", { class: "mrow", role: "listitem", "data-row": m.id }, main, actions);
  if (d) {
    row.dataset.dl = m.id;
    actions.append(h("span", { class: "dl-pct" }, `${d.progress}%`), ringButton(m, d.progress));
  } else {
    const get = h(
      "button",
      { type: "button", class: "get-btn", "aria-label": `Download ${m.name}`, "data-fk": `get:${m.id}` },
      "Get"
    );
    actions.append(onAction(get, () => startDownload(m)));
  }
  errLine(row, m.id);
  return row;
}

// Прогресс скачивания — только кольцо, процент и строка статуса, без
// перерисовки списка (до ~4 событий в секунду на загрузку).
function updateDlRow(row, d) {
  const pct = row.querySelector(".dl-pct");
  const fill = row.querySelector(".ring-fill");
  const meta = row.querySelector(".m-meta");
  if (pct) pct.textContent = `${d.progress}%`;
  if (fill) fill.style.strokeDasharray = dash(d.progress);
  if (meta) meta.textContent = dlMeta(d);
}

function openKeyForm(pid) {
  keyForms[pid] = { value: "", checking: false, enter: true };
  renderOnline();
  const input = document.querySelector(`[data-fk="${CSS.escape(`key:online:${pid}`)}"]`);
  if (input) input.focus();
}
function closeKeyForm(pid) {
  // Пока ключ проверяется, Cancel/Esc не действуют: provider_save_key всё
  // равно доведёт проверку и сохранение до конца.
  if (keyForms[pid] && keyForms[pid].checking) return;
  delete keyForms[pid];
  delete rowErrors[`online:${pid}`];
  renderOnline();
  const add = document.querySelector(`[data-fk="${CSS.escape(`add:online:${pid}`)}"]`);
  if (add) add.focus({ preventScroll: true });
}
function submitKey(p, st) {
  const id = `online:${p.id}`;
  if (st.checking || busy.has(id)) return;
  st.checking = true;
  renderOnline();
  act(id, async () => {
    try {
      await invoke("provider_save_key", { provider: p.id, key: st.value });
      delete keyForms[p.id];
    } finally {
      st.checking = false;
    }
  }).then(() => {
    // ключ отклонён — форма остаётся с введённым значением, выделяем его
    if (keyForms[p.id] !== st) return;
    const input = document.querySelector(`[data-fk="${CSS.escape(`key:${id}`)}"]`);
    if (input) {
      input.focus({ preventScroll: true });
      input.select();
    }
  });
}

function keyFormEls(p, st) {
  const id = `online:${p.id}`;
  const input = h("input", {
    class: "key-input",
    type: "password",
    placeholder: `${p.name} API key`,
    "aria-label": `${p.name} API key`,
    autocomplete: "off",
    autocapitalize: "off",
    spellcheck: "false",
    "data-fk": `key:${id}`,
  });
  input.value = st.value;
  input.addEventListener("input", () => (st.value = input.value));
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeKeyForm(p.id);
    }
  });
  // aria-disabled, а не disabled: фокус остаётся на кнопке (повтор отсекает
  // submitKey по st.checking)
  const off = st.checking ? "true" : null;
  const save = h(
    "button",
    { type: "submit", class: "save-btn", "data-fk": `save:${id}`, "aria-disabled": off },
    st.checking ? "Checking…" : "Save"
  );
  const cancel = h("button", { type: "button", class: "text-btn", "data-fk": `cancel:${id}`, "aria-disabled": off }, "Cancel");
  cancel.addEventListener("click", () => closeKeyForm(p.id));
  const form = h("form", { class: `key-form${st.enter ? " enter" : ""}`, "aria-label": `${p.name} API key` }, input, save, cancel);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    submitKey(p, st);
  });
  const get = h("button", { type: "button", class: "link" }, "Get a key");
  get.addEventListener("click", () => invoke("open_url", { url: p.key_url }).catch(logErr));
  // в макете появляется плавно только строка поля, подпись — сразу
  const note = h("div", { class: "key-note" }, `${IS_MAC ? "Stored in Keychain" : "Stored in Credential Manager"} · `, get);
  st.enter = false;
  return [form, note];
}

function providerRow(p, flips) {
  const id = `online:${p.id}`;
  const on = overview.active === id;
  const form = p.has_key ? null : keyForms[p.id];
  const main = h(
    "div",
    p.has_key
      ? {
          class: "mrow-main",
          role: "radio",
          tabindex: "-1",
          "data-fk": `use:${id}`,
          "aria-labelledby": `name-${id}`,
          "aria-describedby": `langs-${id}`,
        }
      : { class: "mrow-main" },
    p.has_key ? h("span", { class: "radio", "aria-hidden": "true" }) : icon(ICON.key, "lead-icon"),
    h(
      "div",
      null,
      h("div", { class: "p-name" }, h("span", { class: "m-title", id: `name-${id}` }, p.name), h("span", { class: "m-mono" }, p.model)),
      h("div", { class: "p-langs", id: `langs-${id}` }, p.languages_note)
    )
  );
  const actions = h("div", { class: "p-actions" });
  if (on) actions.append(h("span", { class: "in-use" }, "In use"));
  if (p.has_key) {
    actions.append(
      trashButton(`Remove ${p.name} API key`, "Remove key", `del:${id}`, () =>
        act(id, () => invoke("provider_delete_key", { provider: p.id }))
      )
    );
  } else if (!form) {
    const add = h("button", { type: "button", class: "pill sm", "aria-label": `Add ${p.name} API key`, "data-fk": `add:${id}` }, "Add key");
    add.addEventListener("click", (e) => {
      e.stopPropagation();
      openKeyForm(p.id);
    });
    actions.append(add);
  }
  const canUse = p.has_key && !on;
  const row = h(
    "div",
    { class: `mrow prow hoverable${canUse ? " selectable" : ""}`, "data-row": id },
    main,
    h("span", { class: "p-price" }, `~$${p.usd_per_min}/min`),
    actions
  );
  if (p.has_key) withFlip(flips, row, main, on);
  else row.classList.toggle("is-active", on);
  if (form) row.append(...keyFormEls(p, form));
  errLine(row, id);
  if (canUse) activateOn(row, main, () => act(id, () => invoke("model_activate", { id })));
  return row;
}

function discoveredRow(f) {
  return h(
    "div",
    { class: "mrow", role: "listitem" },
    h(
      "div",
      { class: "mrow-main" },
      icon(ICON.sparkle, "lead-icon"),
      h(
        "div",
        null,
        h("div", { class: "m-name" }, h("span", { class: "m-title" }, f.title), h("span", { class: "chip plain" }, "Not rated yet")),
        h(
          "div",
          { class: "m-meta" },
          `${FAMILY_NAMES[f.family] || f.family} family · ${fmtSize(f.size_mb)} · released ${fmtReleased(f.released)}`
        )
      )
    )
  );
}

function fillOnline(flips) {
  const list = $("online-list");
  list.replaceChildren(...overview.providers.map((p) => providerRow(p, flips)));
  fixRoving(list);
}
// Только онлайн-список: открыть/закрыть форму ключа, «Checking…».
function renderOnline() {
  if (!overview) return;
  const list = $("online-list");
  const flips = [];
  keepFocus(list, () => fillOnline(flips));
  runFlips(list, flips);
}

function renderModels() {
  if (!overview) return;
  const lists = $("model-lists");
  const flips = [];
  const installed = overview.models.filter((m) => m.installed && !downloadOf(m));
  const available = overview.models.filter((m) => !m.installed || downloadOf(m));

  keepFocus(lists, () => {
    lists.classList.remove("hidden");
    $("installed-block").classList.toggle("hidden", installed.length === 0);
    $("installed-list").replaceChildren(...installed.map((m) => installedRow(m, flips)));
    fixRoving($("installed-list"));

    $("available-block").classList.toggle("hidden", available.length === 0);
    $("available-list").replaceChildren(...available.map(availableRow));

    fillOnline(flips);
  });

  const found = overview.discovered || [];
  $("discovered-block").classList.toggle("hidden", found.length === 0);
  $("discovered-list").replaceChildren(...found.map(discoveredRow));

  renderUnloadLabel();
  runFlips(lists, flips);
  shownActive = overview.active;
}

$("discovered-link").addEventListener("click", () => {
  if (overview && overview.discovery_url) invoke("open_url", { url: overview.discovery_url }).catch(logErr);
});

function renderUnloadLabel() {
  if (!settings) return;
  const o = UNLOAD_OPTIONS.find((x) => x.value === settings.unload_after_min);
  $("unload-label").textContent = o ? o.name : `After ${settings.unload_after_min} min`;
}
createMenu({
  anchor: $("unload-anchor"),
  trigger: $("unload-trigger"),
  width: 170,
  up: true, // строка внизу страницы — меню вверх, если над ней есть место
  label: "Free memory when idle",
  getItems: () => UNLOAD_OPTIONS.map((o) => ({ ...o, on: o.value === settings.unload_after_min })),
  onPick: (item) => {
    settings.unload_after_min = item.value;
    renderUnloadLabel();
    saveSettings();
  },
});

listen("model-download", (e) => {
  const p = e.payload || {};
  const { id, state } = p;
  if (!id) return;
  if (TERMINAL.has(state)) {
    delete downloads[id];
    if (state === "error") rowErrors[id] = p.message || "Download failed";
    renderStatusCard();
    loadModels();
    return;
  }
  const first = !downloads[id];
  downloads[id] = { progress: p.progress || 0, state, done: p.done_bytes, total: p.total_bytes };
  if (first) renderStatusCard();
  const row = document.querySelector(`[data-dl="${CSS.escape(id)}"]`);
  if (row) updateDlRow(row, downloads[id]);
  else renderModels();
});
listen("models-changed", () => loadModels());

// ============================================================ status card
const STATUS = {
  ready: { label: "Ready", tone: "ok" },
  loading: { label: "Loading…", tone: "warn" },
  downloading: { label: "Downloading…", tone: "warn" },
  unloaded: { label: "Unloaded", tone: "idle", tip: "Unloaded to free memory — it reloads while you speak" },
  error: { label: "Error", tone: "err" },
  missing: { label: "Not ready", tone: "err", tip: "The model is not downloaded yet" },
};

function activeNames(id) {
  if (!id) return ["—", ""];
  if (overview) {
    const m = overview.models.find((x) => x.id === id);
    if (m) return [m.name, m.vendor];
    const p = overview.providers.find((x) => `online:${x.id}` === id);
    if (p) return [`${p.name} ${p.model}`, "Online"];
  }
  if (engine.labelFor === id && engine.name) return [engine.name, engine.vendor];
  return [id, ""];
}

function renderStatusCard() {
  if (!engine.status) return;
  let s = STATUS[engine.status] || STATUS.missing;
  // Первая загрузка модели по умолчанию идёт при статусе missing.
  if (engine.status === "missing" && activeId && isDownloadingId(activeId)) s = STATUS.downloading;
  const [name, vendor] = activeNames(activeId);
  const card = $("status-card");
  card.classList.remove("pending");
  card.dataset.tone = s.tone;
  $("status-label").textContent = s.label;
  $("status-name").textContent = name;
  $("status-vendor").textContent = vendor || " ";
  card.title = engine.status === "error" ? engine.message || "" : s.tip || "";
  card.setAttribute("aria-label", `${name}: ${s.label}. Open Model settings`);
}

// live — событие model-status; иначе ответ get_model_status при открытии,
// который не должен затирать уже пришедшее живое событие.
function applyStatus(st, live) {
  if (live) statusSeq++;
  if (live || !engine.status) {
    engine.status = st.status;
    engine.message = st.message || null;
  }
  if (st.model) activeId = st.model;
  if (st.name) {
    engine.labelFor = st.model;
    engine.name = st.name;
    engine.vendor = st.vendor || "";
  }
  renderStatusCard();
}
listen("model-status", (e) => applyStatus(e.payload || {}, true));
$("status-card").addEventListener("click", () => selectTab("model"));

// ================================================================ history
const KEEP_OPTIONS = [
  { value: "24h", name: "24 hours" },
  { value: "7d", name: "7 days" },
  { value: "30d", name: "30 days" },
  { value: "forever", name: "Forever" },
];
const keepName = (v) => (KEEP_OPTIONS.find((o) => o.value === v) || KEEP_OPTIONS[1]).name;

createMenu({
  anchor: $("keep-anchor"),
  trigger: $("keep-trigger"),
  width: 180,
  head: "Delete after",
  label: "Keep history",
  getItems: () => KEEP_OPTIONS.map((o) => ({ ...o, on: settings.history_keep === o.value })),
  onPick: async (item) => {
    settings.history_keep = item.value;
    $("keep-label").textContent = `Keep: ${item.name}`;
    await saveSettings();
    renderHistory();
  },
});

$("history-clear").addEventListener("click", async () => {
  try {
    await invoke("clear_history");
  } catch (e) {
    logErr(e);
  }
  renderHistory();
});

// Как в макете — только символ клавиши («Hold ⌘ and speak»); у клавиш без
// символа (Fn, F5, клавиши Windows) — её имя.
function renderEmptyHint() {
  const verb = settings && settings.mode === "toggle" ? "Press" : "Hold";
  let key = "the hotkey";
  if (settings) {
    const k = splitKey(settings.hotkey);
    key = k.cap === "⌨" ? k.label : k.cap;
  }
  $("empty-sub").textContent = `${verb} ${key} and speak — entries will appear here`;
}

function dayLabel(ts) {
  const d = new Date(ts);
  const today = new Date();
  const start = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(today) - start(d)) / 86400e3);
  const opts = { month: "short", day: "numeric" };
  if (d.getFullYear() !== today.getFullYear()) opts.year = "numeric";
  const date = d.toLocaleDateString("en-US", opts);
  const weekday = d.toLocaleDateString("en-US", { weekday: "long" });
  if (diff === 0) return ["Today", date];
  if (diff === 1) return ["Yesterday", date];
  if (diff > 1 && diff < 7) return [weekday, date];
  return [date, weekday];
}

let copied = null; // { key, timer } — «Copied» на 1.2 с, переживает перерисовку
function paintCopy(btn, on) {
  btn.classList.toggle("copied", on);
  btn.replaceChildren(...(on ? [icon(ICON.check)] : []), h("span", null, on ? "Copied" : "Copy"));
}
function copyBtn(key) {
  return document.querySelector(`[data-copy="${CSS.escape(key)}"]`);
}
function setCopied(key) {
  if (copied) {
    clearTimeout(copied.timer);
    const old = copyBtn(copied.key);
    if (old) paintCopy(old, false);
  }
  copied = {
    key,
    timer: setTimeout(() => {
      copied = null;
      const b = copyBtn(key);
      if (b) paintCopy(b, false);
    }, 1200),
  };
  const b = copyBtn(key);
  if (b) paintCopy(b, true);
  // смену текста кнопки дикторы не объявляют; пустая строка между
  // повторами — чтобы «Copied» прозвучало и во второй раз
  const live = $("live");
  live.textContent = "";
  requestAnimationFrame(() => (live.textContent = "Copied"));
}

function histRow(it) {
  const key = `${it.ts_ms}:${it.duration_ms}:${(it.text || "").length}`;
  const d = new Date(it.ts_ms);
  const text = it.text || "";
  const btn = h("button", {
    type: "button",
    class: "copy-btn",
    "data-copy": key,
    "data-fk": `copy:${key}`,
    // у каждой строки своё имя, а не N одинаковых «Copy»
    "aria-label": `Copy: ${text.length > 40 ? `${text.slice(0, 40).trimEnd()}…` : text}`,
  });
  paintCopy(btn, !!copied && copied.key === key);
  btn.addEventListener("click", async () => {
    try {
      await invoke("copy_text", { text: it.text });
    } catch (e) {
      logErr(e);
      return;
    }
    setCopied(key);
  });
  return h(
    "div",
    { class: "hrow", role: "listitem" },
    h(
      "div",
      { class: "h-time" },
      h("div", { class: "h-clock" }, d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" })),
      h("div", { class: "h-dur" }, `${((it.duration_ms || 0) / 1000).toFixed(1)} s`)
    ),
    h("div", { class: "h-text" }, it.text),
    btn
  );
}

let histSeq = 0;
async function renderHistory() {
  const my = ++histSeq;
  let items;
  try {
    items = (await invoke("get_history")) || [];
  } catch (e) {
    logErr(e);
    return;
  }
  if (my !== histSeq) return;
  $("history-count").textContent = `${items.length} ${items.length === 1 ? "entry" : "entries"}`;
  $("history-empty").classList.toggle("hidden", items.length > 0);
  const days = new Map();
  for (const it of items) {
    const k = new Date(it.ts_ms).toDateString();
    if (!days.has(k)) days.set(k, { ts: it.ts_ms, items: [] });
    days.get(k).items.push(it);
  }
  const nodes = [];
  for (const g of days.values()) {
    const [day, date] = dayLabel(g.ts);
    nodes.push(h("div", { class: "day-head" }, h("span", { class: "day-name" }, day), h("span", { class: "day-date" }, date)));
    nodes.push(h("div", { class: "group list", role: "list", "aria-label": `${day}, ${date}` }, g.items.map(histRow)));
  }
  const list = $("history-list");
  keepFocus(list, () => list.replaceChildren(...nodes));
}

// =================================================================== tabs
const TABS = [
  { id: "general", label: "General" },
  { id: "record", label: "Recording" },
  { id: "model", label: "Model" },
  { id: "history", label: "History" },
];
const navItems = [...document.querySelectorAll(".nav-item")];

function selectTab(name, focus) {
  const idx = TABS.findIndex((t) => t.id === name);
  if (idx < 0) return;
  closeMenu(false);
  // Захват хоткея — только на General: на других вкладках он молча съедал
  // бы клавиши (например, ввод API-ключа), а чипа «Press keys…» там не видно.
  if (capturing && name !== "general") cancelCapture();
  currentTab = name;
  navItems.forEach((b) => {
    const on = b.dataset.tab === name;
    b.setAttribute("aria-selected", String(on));
    b.tabIndex = on ? 0 : -1;
  });
  $("nav-hl").style.transform = `translateY(${idx * 34}px)`;
  for (const t of TABS) {
    const page = $(`tab-${t.id}`);
    page.classList.toggle("hidden", t.id !== name);
    page.classList.remove("enter");
  }
  const page = $(`tab-${name}`);
  void page.offsetWidth; // перезапуск анимации входа
  page.classList.add("enter");
  $("page-title").textContent = TABS[idx].label;
  $("content").scrollTop = 0;
  if (focus) navItems[idx].focus();
  if (name === "history") renderHistory();
  if (name === "model") loadModels();
}
navItems.forEach((b) => b.addEventListener("click", () => selectTab(b.dataset.tab)));
$("nav").addEventListener("keydown", (e) => {
  const i = navItems.indexOf(document.activeElement);
  if (i < 0) return;
  let n = null;
  if (e.key === "ArrowDown" || e.key === "ArrowRight") n = (i + 1) % navItems.length;
  else if (e.key === "ArrowUp" || e.key === "ArrowLeft") n = (i - 1 + navItems.length) % navItems.length;
  else if (e.key === "Home") n = 0;
  else if (e.key === "End") n = navItems.length - 1;
  if (n === null) return;
  e.preventDefault();
  selectTab(TABS[n].id, true);
});

// ============================================================ permissions
let permPollTimer = null;

function updatePermBanner(p) {
  if (!p) return;
  lastPerm = p;
  const acc = !!p.accessibility;
  const im = !!p.input_monitoring;
  const ok = acc && im;
  $("perm-banner").classList.toggle("hidden", ok);
  $("perm-acc-dot").classList.toggle("ok", acc);
  $("perm-im-dot").classList.toggle("ok", im);
  $("perm-acc-sr").textContent = acc ? " (granted)" : " (not granted)";
  $("perm-im-sr").textContent = im ? " (granted)" : " (not granted)";
  // Одна кнопка, как в макете (две не дают строке статуса уместиться):
  // сначала Accessibility, после её выдачи — Input Monitoring.
  $("open-access").classList.toggle("hidden", acc);
  $("open-input-mon").classList.toggle("hidden", im || !acc);
  renderHotkey(); // кнопка Change недоступна, пока разрешений нет
  clearTimeout(permPollTimer);
  permPollTimer = null;
  // Опрос раз в 3 с, пока чего-то нет и окно видно; событие permissions
  // из Rust приходит и без него.
  if (!ok && document.visibilityState !== "hidden") permPollTimer = setTimeout(refreshPermissions, 3000);
}

async function refreshPermissions() {
  try {
    updatePermBanner(await invoke("permissions_status"));
  } catch (e) {
    logErr(e);
  }
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") {
    clearTimeout(permPollTimer);
    permPollTimer = null;
  } else if (permsMissing()) {
    refreshPermissions();
  }
});

$("open-access").addEventListener("click", () =>
  invoke("open_permission_settings", { which: "accessibility" }).catch(logErr)
);
$("open-input-mon").addEventListener("click", () =>
  invoke("open_permission_settings", { which: "input-monitoring" }).catch(logErr)
);

// ================================================================= events
listen("history-updated", () => {
  if (currentTab === "history") renderHistory();
});
listen("permissions", (e) => updatePermBanner(e.payload));

// Пока идёт захват, клавиши уходят в Rust и на странице ничего не нажимают
// (иначе Space/Enter заодно «нажали» бы кнопку в фокусе). Esc — отмена.
let swallowKeyUp = false;
document.addEventListener(
  "keydown",
  (e) => {
    if (capturing) {
      e.preventDefault();
      if (e.key === "Escape") cancelCapture();
      else swallowKeyUp = true;
      return;
    }
    if (e.key === "Escape" && openMenu) {
      e.preventDefault();
      closeMenu(true);
    }
  },
  true
);
document.addEventListener(
  "keyup",
  (e) => {
    if (!swallowKeyUp) return;
    swallowKeyUp = false;
    e.preventDefault();
  },
  true
);
window.addEventListener("blur", () => closeMenu(false));
// Страница уходит с включённым захватом — снимаем флаг, иначе следующая
// клавиша в любом приложении стала бы хоткеем (Rust страхует то же при
// уничтожении окна).
window.addEventListener("pagehide", () => {
  if (capturing) invoke("cancel_hotkey_capture").catch(() => {});
});

// =================================================================== init
function applySettings() {
  renderHotkey();
  renderMicLabel();
  $("sounds-toggle").setAttribute("aria-checked", String(!!settings.sounds));
  $("autostart-toggle").setAttribute("aria-checked", String(!!settings.autostart));
  $("keep-label").textContent = `Keep: ${keepName(settings.history_keep)}`;
  renderMode();
  minSlider.render();
  maxSlider.render();
  renderUnloadLabel();
}

async function loadVersion() {
  let v = null;
  if (IS_TAURI) {
    try {
      v = await window.__TAURI__.app.getVersion();
    } catch (e) {
      logErr(e);
    }
  } else {
    v = DEMO.version;
  }
  if (v) $("version").textContent = `Version ${v}`;
}

renderHotkey(); // имя клавиши по умолчанию для платформы, пока нет настроек

(async function init() {
  // Флаг захвата мог остаться от прошлой страницы (перезагрузка) — снимаем.
  if (IS_TAURI) invoke("cancel_hotkey_capture").catch(() => {});
  try {
    settings = await invoke("get_settings");
  } catch (e) {
    logErr(e);
  }
  if (settings) applySettings();
  requestAnimationFrame(() =>
    requestAnimationFrame(() => document.documentElement.classList.remove("booting"))
  );
  invoke("list_mics")
    .then((list) => {
      mics = Array.isArray(list) ? list : [];
    })
    .catch(logErr);
  loadVersion();
  // История (≤ 50 записей) — заранее, чтобы вкладка открывалась готовой.
  // models_overview заранее не зовём: он читает Связку ключей, а это может
  // вызвать системный запрос прямо при открытии окна.
  renderHistory();
  try {
    const st = await invoke("get_model_status");
    if (st) applyStatus(st, false);
  } catch (e) {
    logErr(e);
  }
  await refreshPermissions();
  if (DEMO) DEMO.ready();
})();

// =================================================================== demo
// Превью без Tauri: `python3 -m http.server 4173 --directory ui`, затем
// http://localhost:4173/settings.html. Команды имитируют бэкенд (ошибки —
// строками, как у Tauri), события приходят асинхронно. Параметры адреса:
// ?tab=model|history|record  ?perm=0|acc|im (каких разрешений нет).
// Отладка из консоли: window.__settingsDebug.
function createDemo() {
  const q = new URLSearchParams(location.search);
  const mac = navigator.platform.toUpperCase().includes("MAC");
  const handlers = {};
  const emit = (event, payload) =>
    (handlers[event] || []).slice().forEach((fn) => {
      try {
        fn({ event, payload });
      } catch (e) {
        console.error(e);
      }
    });
  const later = (event, payload, ms = 0) => setTimeout(() => emit(event, payload), ms);
  const clone = (v) => (v === undefined || v === null ? v : JSON.parse(JSON.stringify(v)));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const now = Date.now();

  const settings = {
    hotkey: mac ? "MetaRight" : "ControlRight",
    mode: "ptt",
    min_duration_ms: 300,
    max_record_s: 120,
    sounds: true,
    mic_device: null,
    autostart: false,
    history_keep: "7d",
    active_model: "parakeet-tdt-0.6b-v3",
    unload_after_min: 5,
  };
  const model = (id, name, vendor, label, languages, note, en, ram, size, dl, speed, installed) => ({
    id,
    name,
    vendor,
    labels: [label],
    languages,
    languages_note: note,
    english_only: en,
    ram_mb: ram,
    size_mb: size,
    speed,
    download_mb: dl,
    license: "",
    homepage: "",
    installed,
    downloading: false,
  });
  const overview = {
    active: "parakeet-tdt-0.6b-v3",
    status: "ready",
    unload_after_min: 5,
    models: [
      model("parakeet-tdt-0.6b-v3", "Parakeet TDT 0.6B v3", "NVIDIA", "best_balance", 25, "25 European languages, auto-detected", false, 1200, 670, 670, "fast", true),
      model("whisper-large-v3-turbo", "Whisper large-v3 turbo", "OpenAI", "most_languages", 99, "99 languages, auto-detected", false, 1900, 1037, 1037, "slow", false),
      model("qwen3-asr-0.6b", "Qwen3-ASR 0.6B", "Alibaba Qwen", "asian_languages", 30, "30 languages incl. Chinese, Japanese, Korean, Arabic, Hindi", false, 1800, 987, 987, "medium", false),
      model("parakeet-tdt-110m-en", "Parakeet TDT 110M", "NVIDIA", "fastest", 1, "English only", true, 300, 136, 108, "fast", true),
      model("moonshine-v2-tiny-en", "Moonshine v2 Tiny", "Useful Sensors", "smallest", 1, "English only", true, 100, 44, 44, "fast", false),
    ],
    providers: [
      { id: "openai", name: "OpenAI", model: "gpt-transcribe", usd_per_min: 0.0045, languages_note: "Dozens of languages", key_url: "https://platform.openai.com/api-keys", has_key: false },
      { id: "groq", name: "Groq", model: "whisper-large-v3-turbo", usd_per_min: 0.00067, languages_note: "99 languages", key_url: "https://console.groq.com/keys", has_key: true },
      { id: "elevenlabs", name: "ElevenLabs", model: "scribe_v2", usd_per_min: 0.0037, languages_note: "90+ languages", key_url: "https://elevenlabs.io/app/settings/api-keys", has_key: false },
    ],
    discovered: [{ title: "parakeet tdt 0.6b v4 int8", family: "nemo_transducer", size_mb: 693, released: "2026-09-24" }],
    discovery_url: "https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models",
  };
  const history = [
    { text: "Testing voice input. One, two, three.", ts_ms: now - 3600e3, duration_ms: 4200 },
    { text: "The weather is great today, let's take a walk in the park after lunch and grab a coffee on the way back.", ts_ms: now - 7200e3, duration_ms: 12400 },
    { text: "Remind me to send the design handoff to the team tomorrow morning.", ts_ms: now - 86400e3, duration_ms: 6100 },
  ];
  const missing = q.get("perm") || "";
  const perms = {
    accessibility: !["0", "all", "acc"].includes(missing),
    input_monitoring: !["0", "all", "im"].includes(missing),
  };

  const find = (id) => overview.models.find((m) => m.id === id);
  const provider = (id) => overview.providers.find((p) => p.id === id);
  const bytes = (m) => m.download_mb * 1e6;
  const ARCHIVE = new Set(["parakeet-tdt-110m-en"]);
  const jobs = {}; // id → { timer } | { stalled }

  function progress(m, state, done) {
    emit("model-download", {
      id: m.id,
      state,
      progress: Math.min(100, Math.floor((done * 100) / bytes(m))),
      done_bytes: Math.round(done),
      total_bytes: bytes(m),
      message: null,
    });
  }
  function setStatus(status, message = null) {
    overview.status = status;
    emit("model-status", { status, model: overview.active, message });
  }
  function usable(id) {
    if (id.startsWith("online:")) return !!(provider(id.slice(7)) || {}).has_key;
    return !!(find(id) || {}).installed;
  }
  function reload() {
    setStatus("loading");
    setTimeout(() => setStatus(usable(overview.active) ? "ready" : "missing"), 700);
  }
  function finish(m, state, message) {
    const job = jobs[m.id];
    if (job && job.timer) clearTimeout(job.timer);
    delete jobs[m.id];
    m.downloading = false;
    if (state === "done") m.installed = true;
    emit("model-download", {
      id: m.id,
      state,
      progress: state === "done" ? 100 : 0,
      done_bytes: state === "done" ? bytes(m) : 0,
      total_bytes: bytes(m),
      message: message || null,
    });
    emit("models-changed", null);
    if (state === "done" && overview.active === m.id) reload();
  }
  function startJob(m) {
    if (jobs[m.id]) return; // как в Rust: повторный старт — тихий no-op
    m.downloading = true;
    const job = (jobs[m.id] = {});
    let done = 0;
    const tick = () => {
      done = Math.min(bytes(m), done + bytes(m) * (0.03 + Math.random() * 0.05));
      if (done < bytes(m)) {
        progress(m, "downloading", done);
        job.timer = setTimeout(tick, 250);
        return;
      }
      progress(m, "verifying", done);
      job.timer = setTimeout(() => {
        if (ARCHIVE.has(m.id)) progress(m, "extracting", done);
        job.timer = setTimeout(() => finish(m, "done"), ARCHIVE.has(m.id) ? 700 : 0);
      }, 800);
    };
    job.timer = setTimeout(tick, 250);
    later("models-changed", null);
  }
  // «зависшая» загрузка из макета: Qwen3 на 42 %
  const STALLED = "qwen3-asr-0.6b";
  find(STALLED).downloading = true;
  jobs[STALLED] = { stalled: true };

  const CODE_NAMES = {
    MetaRight: "MetaRight",
    MetaLeft: "MetaLeft",
    OSRight: "MetaRight",
    OSLeft: "MetaLeft",
    ControlLeft: "ControlLeft",
    ControlRight: "ControlRight",
    ShiftLeft: "ShiftLeft",
    ShiftRight: "ShiftRight",
    AltLeft: "Alt",
    AltRight: "AltGr",
    CapsLock: "CapsLock",
    Fn: "Function",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
    Insert: "Insert",
  };
  let armed = null;
  function arm() {
    disarm();
    armed = (e) => {
      if (e.repeat) return;
      disarm();
      if (e.code === "Escape") return later("hotkey-captured", { cancelled: true });
      const name = CODE_NAMES[e.code] || (/^F\d+$/.test(e.code) ? e.code : `Key0x${(e.keyCode || 0).toString(16).toUpperCase()}`);
      settings.hotkey = name;
      later("hotkey-captured", { key: name });
    };
    window.addEventListener("keydown", armed, true);
  }
  function disarm() {
    if (armed) window.removeEventListener("keydown", armed, true);
    armed = null;
  }

  const ALLOWED = ["huggingface.co", "github.com", "www.nvidia.com", "platform.openai.com", "console.groq.com", "elevenlabs.io"];
  const commands = {
    get_settings: () => settings,
    set_settings: ({ newSettings }) => {
      const keep = { hotkey: settings.hotkey, active_model: settings.active_model };
      Object.assign(settings, clone(newSettings), keep);
    },
    list_mics: () => ["MacBook Pro Microphone", "AirPods Pro"],
    get_history: () => history,
    clear_history: () => {
      history.length = 0;
      later("history-updated", null);
    },
    copy_text: ({ text }) => {
      if (navigator.clipboard) navigator.clipboard.writeText(text).catch(() => {});
    },
    get_model_status: () => {
      const id = overview.active;
      const m = find(id);
      const p = id.startsWith("online:") && provider(id.slice(7));
      return {
        status: overview.status,
        model: id,
        name: m ? m.name : p ? `${p.name} ${p.model}` : id,
        vendor: m ? m.vendor : p ? "Online" : "",
      };
    },
    models_overview: () => {
      if (jobs[STALLED] && jobs[STALLED].stalled) {
        setTimeout(() => progress(find(STALLED), "downloading", bytes(find(STALLED)) * 0.42), 0);
      }
      return overview;
    },
    model_download: ({ id }) => {
      const m = find(id);
      if (!m) throw "Unknown model";
      startJob(m);
    },
    model_cancel_download: ({ id }) => {
      const m = find(id);
      if (m && jobs[id]) finish(m, "cancelled");
    },
    model_delete: ({ id }) => {
      const m = find(id);
      if (!m) throw "Unknown model";
      if (overview.active === id) throw "Switch to another model before deleting this one";
      m.installed = false;
      later("models-changed", null);
    },
    model_activate: ({ id }) => {
      if (id.startsWith("online:")) {
        const p = provider(id.slice(7));
        if (!p) throw "Unknown provider";
        if (!p.has_key) throw `Add an API key for ${p.name} first`;
      } else {
        const m = find(id);
        if (!m) throw "Unknown model";
        if (!m.installed) throw "Download the model first";
      }
      overview.active = id;
      settings.active_model = id;
      later("models-changed", null);
      reload();
    },
    provider_save_key: async ({ provider: pid, key }) => {
      const p = provider(pid);
      if (!p) throw "Unknown provider";
      const k = (key || "").trim();
      if (!k) throw "Paste an API key";
      await sleep(900);
      if (k.length < 8) throw `${p.name} rejected this API key`;
      p.has_key = true;
      later("models-changed", null);
    },
    provider_delete_key: ({ provider: pid }) => {
      const p = provider(pid);
      if (!p) throw "Unknown provider";
      p.has_key = false;
      if (overview.active === `online:${pid}`) {
        const first = overview.models.find((m) => m.installed);
        overview.active = settings.active_model = first ? first.id : "parakeet-tdt-0.6b-v3";
        reload();
      }
      later("models-changed", null);
    },
    begin_hotkey_capture: () => arm(),
    cancel_hotkey_capture: () => disarm(),
    permissions_status: () => perms,
    open_permission_settings: ({ which }) => {
      // имитация: пользователь выдал разрешение в Системных настройках
      setTimeout(() => {
        if (which === "input-monitoring") perms.input_monitoring = true;
        else perms.accessibility = true;
        emit("permissions", clone(perms));
      }, 1500);
    },
    open_url: ({ url }) => {
      const host = /^https:\/\/([^/?#]+)/.exec(url || "");
      if (!host || !ALLOWED.includes(host[1].toLowerCase())) throw "url not allowed";
      console.info("[demo] open_url", url);
    },
  };

  // Сцена из макета: «обои» за стеклом и нарисованные «светофоры».
  const win = document.getElementById("window");
  const stage = document.createElement("div");
  stage.className = "demo-stage";
  [
    [-70, -50, 330, "#ff4f7b"],
    [520, -90, 260, "#1fb6a8"],
    [430, 250, 380, "#4b5bff"],
    [150, 390, 250, "#9d4dff"],
  ].forEach(([l, t, s, c]) => {
    const b = document.createElement("div");
    b.className = "demo-blob";
    b.style.cssText = `left:${l}px;top:${t}px;width:${s}px;height:${s}px;background:${c}`;
    stage.append(b);
  });
  win.parentNode.insertBefore(stage, win);
  stage.append(win);
  const lights = document.createElement("div");
  lights.className = "demo-lights";
  lights.setAttribute("aria-hidden", "true");
  lights.append(document.createElement("span"), document.createElement("span"), document.createElement("span"));
  win.append(lights);

  window.__settingsDebug = {
    emit,
    perms(accessibility, input_monitoring) {
      Object.assign(perms, { accessibility, input_monitoring });
      emit("permissions", clone(perms));
    },
    status: (status, message) => setStatus(status, message || null),
    download: (id) => commands.model_download({ id }),
    fail(id, message) {
      const m = find(id);
      if (m) finish(m, "error", message || "файл encoder.int8.onnx: запрос https://huggingface.co/…: соединение сброшено");
    },
    addHistory(text, agoMs = 0) {
      history.unshift({ text, ts_ms: Date.now() - agoMs, duration_ms: 3200 });
      later("history-updated", null);
    },
    tab: (t) => selectTab(t),
    state: { settings, overview, history, perms },
  };

  return {
    version: "0.1.0",
    invoke: async (cmd, args) => {
      const f = commands[cmd];
      return f ? clone(await f(args || {})) : undefined;
    },
    listen: (event, fn) => {
      (handlers[event] = handlers[event] || []).push(fn);
    },
    ready() {
      const tab = q.get("tab");
      if (tab) selectTab(tab);
    },
  };
}
