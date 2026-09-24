// Вне Tauri (превью в браузере) — демо-режим с моками, чтобы верстать UI.
const IS_TAURI = !!window.__TAURI__;
const DEMO = {
  settings: {
    hotkey: "MetaRight",
    mode: "ptt",
    min_duration_ms: 300,
    max_record_s: 120,
    sounds: true,
    mic_device: null,
    autostart: false,
    history_keep: "7d",
  },
  history: [
    { text: "Testing voice input. One, two, three.", ts_ms: Date.now() - 3600e3, duration_ms: 4200 },
    { text: "The weather is great today, let's take a walk in the park after lunch and grab a coffee on the way back.", ts_ms: Date.now() - 7200e3, duration_ms: 12400 },
    { text: "Remind me to send the design handoff to the team tomorrow morning.", ts_ms: Date.now() - 86400e3, duration_ms: 6100 },
  ],
};
const { invoke } = IS_TAURI
  ? window.__TAURI__.core
  : {
      invoke: async (cmd) =>
        ({
          get_settings: DEMO.settings,
          list_mics: ["MacBook Pro Microphone", "AirPods Pro"],
          get_history: DEMO.history,
          get_model_status: { status: "ready" },
          permissions_status: { accessibility: true, input_monitoring: true },
        })[cmd],
    };
const { listen } = IS_TAURI ? window.__TAURI__.event : { listen: () => {} };

let settings = null;

function $(id) {
  return document.getElementById(id);
}

async function saveSettings() {
  await invoke("set_settings", { newSettings: settings });
}

// ------------------------------------------------------------ key names
const IS_MAC = navigator.platform.toUpperCase().includes("MAC");
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

function keyDisplay(name) {
  if (KEY_NAMES[name]) return KEY_NAMES[name];
  if (/^F\d+$/.test(name)) return name;
  const m = name.match(/^Key0x([0-9A-F]+)$/i);
  if (m) return `Key 0x${m[1]}`;
  return name;
}

// ------------------------------------------------------------ dropdowns
// Общий компонент меню по дизайну: #2a2e38, r14, menuIn 220ms.
function createDropdown({ anchor, trigger, width, section, getItems, onPick }) {
  let menu = null;

  function close() {
    if (menu) {
      menu.remove();
      menu = null;
      document.removeEventListener("pointerdown", onOutside, true);
    }
  }
  function onOutside(e) {
    if (menu && !menu.contains(e.target) && !trigger.contains(e.target)) close();
  }
  trigger.addEventListener("click", () => {
    if (menu) return close();
    menu = document.createElement("div");
    menu.className = "dd-menu";
    if (width) menu.style.width = `${width}px`;
    if (section) {
      const s = document.createElement("div");
      s.className = "dd-section";
      s.textContent = section;
      menu.appendChild(s);
    }
    for (const item of getItems()) {
      const el = document.createElement("div");
      el.className = "dd-item" + (item.on ? " on" : "");
      el.textContent = item.name;
      if (item.on) {
        el.insertAdjacentHTML(
          "beforeend",
          '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#a3a3f7" stroke-width="2.6" stroke-linecap="round"><path d="M4.5 12.5l5 5 10-11"/></svg>'
        );
      }
      el.onclick = () => {
        close();
        onPick(item);
      };
      menu.appendChild(el);
    }
    anchor.appendChild(menu);
    document.addEventListener("pointerdown", onOutside, true);
  });
  return { close };
}

// микрофон
let mics = [];
createDropdown({
  anchor: $("mic-anchor"),
  trigger: $("mic-trigger"),
  width: 240,
  getItems: () => [
    { name: "System default", value: null, on: !settings.mic_device },
    ...mics.map((m) => ({ name: m, value: m, on: settings.mic_device === m })),
  ],
  onPick: (item) => {
    settings.mic_device = item.value;
    $("mic-label").textContent = item.name;
    saveSettings();
  },
});

// срок хранения истории
const KEEP_OPTIONS = [
  { value: "24h", name: "24 hours" },
  { value: "7d", name: "7 days" },
  { value: "30d", name: "30 days" },
  { value: "forever", name: "Forever" },
];
const keepName = (v) => (KEEP_OPTIONS.find((o) => o.value === v) || KEEP_OPTIONS[1]).name;
createDropdown({
  anchor: $("keep-anchor"),
  trigger: $("keep-trigger"),
  width: 190,
  section: "Delete after",
  getItems: () => KEEP_OPTIONS.map((o) => ({ ...o, on: settings.history_keep === o.value })),
  onPick: async (item) => {
    settings.history_keep = item.value;
    $("keep-label").textContent = `Keep: ${item.name}`;
    await saveSettings();
    renderHistory();
  },
});

// ------------------------------------------------------------- segmented
const MODES = ["ptt", "toggle", "dynamic"];
function renderMode() {
  const idx = Math.max(0, MODES.indexOf(settings.mode));
  document.querySelectorAll("#mode-seg button").forEach((b) => {
    b.classList.toggle("active", b.dataset.mode === settings.mode);
  });
  document.querySelector("#mode-seg .seg-thumb").style.transform = `translateX(${idx * 78}px)`;
}
document.querySelectorAll("#mode-seg button").forEach((b) => {
  b.onclick = () => {
    settings.mode = b.dataset.mode;
    renderMode();
    saveSettings();
  };
});

// --------------------------------------------------------------- sliders
function attachSlider(el, valueEl, fmt, getVal, setVal) {
  const min = +el.dataset.min;
  const max = +el.dataset.max;
  const step = +el.dataset.step;

  function render() {
    const v = getVal();
    const pct = ((v - min) / (max - min)) * 100;
    el.querySelector(".fill").style.width = `${pct}%`;
    el.querySelector(".knob").style.left = `${pct}%`;
    valueEl.textContent = fmt(v);
  }
  function fromEvent(e) {
    const rect = el.getBoundingClientRect();
    const t = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    const raw = min + t * (max - min);
    return Math.round(raw / step) * step;
  }
  let dragging = false;
  el.addEventListener("pointerdown", (e) => {
    dragging = true;
    el.setPointerCapture(e.pointerId);
    setVal(fromEvent(e));
    render();
  });
  el.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    setVal(fromEvent(e));
    render();
  });
  el.addEventListener("pointerup", () => {
    dragging = false;
    saveSettings();
  });
  return { render };
}

const minSlider = attachSlider(
  $("minlen-slider"),
  $("minlen-val"),
  (v) => `${v} ms`,
  () => settings.min_duration_ms,
  (v) => (settings.min_duration_ms = v)
);
const maxSlider = attachSlider(
  $("maxlen-slider"),
  $("maxlen-val"),
  (v) => `${v} s`,
  () => settings.max_record_s,
  (v) => (settings.max_record_s = v)
);

// -------------------------------------------------------------- toggles
$("sounds-toggle").onchange = (e) => {
  settings.sounds = e.target.checked;
  saveSettings();
};
$("autostart-toggle").onchange = (e) => {
  settings.autostart = e.target.checked;
  saveSettings();
};

// -------------------------------------------------------- hotkey capture
let capturing = false;
$("hotkey-change").onclick = async () => {
  if (capturing) {
    capturing = false;
    $("hotkey-label").classList.remove("capturing");
    $("hotkey-label").textContent = keyDisplay(settings.hotkey);
    $("hotkey-change").textContent = "Change";
    await invoke("cancel_hotkey_capture");
    return;
  }
  capturing = true;
  $("hotkey-label").classList.add("capturing");
  $("hotkey-label").textContent = "Press keys…";
  $("hotkey-change").textContent = "Cancel";
  await invoke("begin_hotkey_capture");
};
listen("hotkey-captured", (e) => {
  capturing = false;
  $("hotkey-label").classList.remove("capturing");
  $("hotkey-change").textContent = "Change";
  if (!e.payload.cancelled && e.payload.key) {
    settings.hotkey = e.payload.key;
  }
  $("hotkey-label").textContent = keyDisplay(settings.hotkey);
});

// ---------------------------------------------------------------- model
function setModelStatus(status, payload = {}) {
  const pill = $("model-pill");
  const label = $("model-pill-label");
  pill.classList.remove("err", "busy");
  switch (status) {
    case "ready":
      label.textContent = "Enabled";
      break;
    case "loading":
      pill.classList.add("busy");
      label.textContent = "Loading…";
      break;
    case "downloading":
      pill.classList.add("busy");
      label.textContent = `Downloading ${payload.progress ?? 0}%`;
      break;
    case "error":
      pill.classList.add("err");
      label.textContent = "Error";
      break;
    default:
      pill.classList.add("err");
      label.textContent = "Not downloaded";
  }
}

// -------------------------------------------------------------- history
function formatEntryTime(tsMs, durationMs) {
  const d = new Date(tsMs);
  const date = d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
  return `${date} at ${time} · ${(durationMs / 1000).toFixed(1)} s`;
}

async function renderHistory() {
  const items = await invoke("get_history");
  const list = $("history-list");
  list.innerHTML = "";
  $("history-empty").classList.toggle("hidden", items.length > 0);
  $("history-count").textContent = `${items.length} ${items.length === 1 ? "entry" : "entries"}`;

  items.forEach((item, i) => {
    const card = document.createElement("div");
    card.className = "hist-card";
    card.style.animationDelay = `${i * 55}ms`;

    const body = document.createElement("div");
    const text = document.createElement("div");
    text.className = "hist-text";
    text.textContent = item.text;
    const time = document.createElement("div");
    time.className = "hist-time";
    time.textContent = formatEntryTime(item.ts_ms, item.duration_ms);
    body.appendChild(text);
    body.appendChild(time);

    const btn = document.createElement("button");
    btn.className = "btn copy-btn";
    btn.textContent = "Copy";
    btn.onclick = async () => {
      await invoke("copy_text", { text: item.text });
      btn.textContent = "Copied";
      btn.classList.add("copied");
      setTimeout(() => {
        btn.textContent = "Copy";
        btn.classList.remove("copied");
      }, 1200);
    };

    card.appendChild(body);
    card.appendChild(btn);
    list.appendChild(card);
  });
}

$("history-clear").onclick = async () => {
  await invoke("clear_history");
  renderHistory();
};

// ----------------------------------------------------------------- tabs
document.querySelectorAll(".tab").forEach((tab) => {
  tab.onclick = () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    document.querySelectorAll(".tab-page").forEach((p) => p.classList.add("hidden"));
    const page = $(`tab-${tab.dataset.tab}`);
    page.classList.remove("hidden");
    // перезапуск анимации входа
    page.style.animation = "none";
    void page.offsetWidth;
    page.style.animation = "";
    if (tab.dataset.tab === "history") renderHistory();
  };
});

// ---------------------------------------------------------- permissions
let permPollTimer = null;

function updatePermBanner(p) {
  const ok = p.accessibility && p.input_monitoring;
  $("perm-banner").classList.toggle("hidden", ok);
  $("perm-acc-dot").classList.toggle("ok", !!p.accessibility);
  $("perm-im-dot").classList.toggle("ok", !!p.input_monitoring);
  clearTimeout(permPollTimer);
  if (!ok) permPollTimer = setTimeout(refreshPermissions, 3000);
}

async function refreshPermissions() {
  updatePermBanner(await invoke("permissions_status"));
}

$("open-access").onclick = () => invoke("open_permission_settings", { which: "accessibility" });
$("open-input-mon").onclick = () => invoke("open_permission_settings", { which: "input-monitoring" });

// --------------------------------------------------------------- events
listen("model-status", (e) => setModelStatus(e.payload.status, e.payload));
listen("history-updated", () => {
  if (!$("tab-history").classList.contains("hidden")) renderHistory();
});
listen("permissions", (e) => updatePermBanner(e.payload));

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && capturing) $("hotkey-change").click();
});

// ----------------------------------------------------------------- init
(async function init() {
  settings = await invoke("get_settings");
  mics = await invoke("list_mics");

  if (!IS_MAC) {
    document.querySelector(".empty-sub").textContent =
      "Hold the hotkey and speak — entries will appear here";
  }

  $("hotkey-label").textContent = keyDisplay(settings.hotkey);
  $("mic-label").textContent = settings.mic_device || "System default";
  $("sounds-toggle").checked = settings.sounds;
  $("autostart-toggle").checked = settings.autostart;
  $("keep-label").textContent = `Keep: ${keepName(settings.history_keep)}`;
  renderMode();
  minSlider.render();
  maxSlider.render();

  const model = await invoke("get_model_status");
  setModelStatus(model.status);
  await refreshPermissions();
})();
