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
    active_model: "parakeet-tdt-0.6b-v3",
    unload_after_min: 5,
  },
  overview: {
    active: "parakeet-tdt-0.6b-v3",
    status: "ready",
    unload_after_min: 5,
    models: [
      { id: "parakeet-tdt-0.6b-v3", name: "Parakeet TDT 0.6B v3", vendor: "NVIDIA", labels: ["best_balance"], languages_note: "25 European languages, auto-detected", english_only: false, ram_mb: 1200, size_mb: 670, download_mb: 670, installed: true, downloading: false },
      { id: "whisper-large-v3-turbo", name: "Whisper large-v3 turbo", vendor: "OpenAI", labels: ["most_languages"], languages_note: "99 languages, auto-detected", english_only: false, ram_mb: 1900, size_mb: 1037, download_mb: 1037, installed: false, downloading: false },
      { id: "qwen3-asr-0.6b", name: "Qwen3-ASR 0.6B", vendor: "Alibaba Qwen", labels: ["asian_languages"], languages_note: "30 languages incl. Chinese, Japanese, Korean, Arabic, Hindi", english_only: false, ram_mb: 1800, size_mb: 987, download_mb: 987, installed: false, downloading: true },
      { id: "parakeet-tdt-110m-en", name: "Parakeet TDT 110M", vendor: "NVIDIA", labels: ["fastest"], languages_note: "English only", english_only: true, ram_mb: 300, size_mb: 136, download_mb: 108, installed: true, downloading: false },
      { id: "moonshine-v2-tiny-en", name: "Moonshine v2 Tiny", vendor: "Useful Sensors", labels: ["smallest"], languages_note: "English only", english_only: true, ram_mb: 100, size_mb: 44, download_mb: 44, installed: false, downloading: false },
    ],
    providers: [
      { id: "openai", name: "OpenAI", model: "gpt-transcribe", usd_per_min: 0.0045, languages_note: "Dozens of languages", key_url: "https://platform.openai.com/api-keys", has_key: false },
      { id: "groq", name: "Groq", model: "whisper-large-v3-turbo", usd_per_min: 0.00067, languages_note: "99 languages", key_url: "https://console.groq.com/keys", has_key: true },
      { id: "elevenlabs", name: "ElevenLabs", model: "scribe_v2", usd_per_min: 0.0037, languages_note: "90+ languages", key_url: "https://elevenlabs.io/app/settings/api-keys", has_key: false },
    ],
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
          models_overview: DEMO.overview,
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

// ---------------------------------------------------------------- models
const LABELS = {
  best_balance: "Best balance",
  most_languages: "Most languages",
  asian_languages: "Asian languages",
  fastest: "Fastest",
  smallest: "Smallest",
};
const UNLOAD_OPTIONS = [
  { value: 0, name: "Never" },
  { value: 2, name: "After 2 min" },
  { value: 5, name: "After 5 min" },
  { value: 15, name: "After 15 min" },
  { value: 30, name: "After 30 min" },
];

let overview = null;
let engineStatus = { status: "missing" };
const rowErrors = {}; // id → текст ошибки под строкой
const openKeyForms = new Set(); // провайдеры с открытой формой ключа
const downloads = {}; // id → { progress, state }

const fmtSize = (mb) => (mb >= 1000 ? `${(mb / 1000).toFixed(2)} GB` : `${Math.round(mb)} MB`);
const fmtRam = (mb) => (mb >= 1000 ? `~${(mb / 1000).toFixed(1)} GB RAM` : `~${mb} MB RAM`);

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function button(text, onClick, cls = "btn") {
  const b = el("button", cls, text);
  b.type = "button";
  b.onclick = onClick;
  return b;
}

const TRASH_SVG =
  '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/></svg>';

async function act(id, fn) {
  delete rowErrors[id];
  try {
    await fn();
  } catch (e) {
    rowErrors[id] = String(e);
  }
  await loadModels();
}

function activePill() {
  const pill = el("div", "status-pill");
  const dot = el("span", "dot");
  const label = el("span", null, "Active");
  switch (engineStatus.status) {
    case "loading":
      pill.classList.add("busy");
      label.textContent = "Loading…";
      break;
    case "unloaded":
      label.textContent = "Active · sleeping";
      break;
    case "error":
      pill.classList.add("err");
      label.textContent = "Error";
      break;
    case "downloading":
    case "missing":
      pill.classList.add("busy");
      label.textContent = "Not ready";
      break;
    default:
      dot.classList.add("pulse");
  }
  pill.append(dot, label);
  return pill;
}

function downloadBox(m) {
  const d = downloads[m.id] || { progress: 0, state: "downloading" };
  const box = el("div", "dl-box");
  const track = el("div", "dl-track");
  const fill = el("div", "dl-fill");
  fill.style.width = `${d.progress}%`;
  track.append(fill);
  const stateText = { verifying: "Checking…", extracting: "Unpacking…" }[d.state];
  const pct = el("span", "dl-pct", stateText || `${d.progress}%`);
  box.dataset.dl = m.id;
  box.append(track, pct);
  return box;
}

function renderLocal() {
  const list = $("local-models");
  list.replaceChildren();
  for (const m of overview.models) {
    const isActive = overview.active === m.id;
    const row = el("div", "model-row" + (isActive ? " active" : ""));
    const main = el("div", "model-main");

    const name = el("div", "model-name", m.name);
    name.append(el("span", "model-vendor", m.vendor));
    for (const l of m.labels) name.append(el("span", "chip", LABELS[l] || l));
    if (m.speed === "slow") name.append(el("span", "chip plain", "Slower"));
    main.append(name);

    const size = m.installed ? fmtSize(m.size_mb) : `${fmtSize(m.download_mb)} download`;
    main.append(el("div", "model-meta", `${m.languages_note} · ${size} · ${fmtRam(m.ram_mb)}`));
    if (rowErrors[m.id]) main.append(el("div", "model-error", rowErrors[m.id]));

    const actions = el("div", "model-actions");
    if (m.downloading || downloads[m.id]) {
      actions.append(downloadBox(m));
      actions.append(
        button("Cancel", () => act(m.id, () => invoke("model_cancel_download", { id: m.id })))
      );
    } else if (isActive) {
      actions.append(activePill());
    } else if (m.installed) {
      actions.append(button("Use", () => act(m.id, () => invoke("model_activate", { id: m.id }))));
      const del = button("", () => act(m.id, () => invoke("model_delete", { id: m.id })), "btn icon-btn");
      del.setAttribute("aria-label", `Delete ${m.name}`);
      del.title = "Delete from this Mac";
      del.insertAdjacentHTML("beforeend", TRASH_SVG);
      actions.append(del);
    } else {
      actions.append(
        button("Download", () => {
          downloads[m.id] = { progress: 0, state: "downloading" };
          return act(m.id, () => invoke("model_download", { id: m.id }));
        })
      );
    }
    row.append(main, actions);
    list.append(row);
  }
}

function renderOnline() {
  const list = $("online-providers");
  list.replaceChildren();
  for (const p of overview.providers) {
    const id = `online:${p.id}`;
    const isActive = overview.active === id;
    const row = el("div", "model-row" + (isActive ? " active" : ""));
    row.style.flexWrap = "wrap";
    const main = el("div", "model-main");
    const name = el("div", "model-name", p.name);
    name.append(el("span", "model-vendor", p.model));
    main.append(name);
    main.append(el("div", "model-meta", `${p.languages_note} · ~$${p.usd_per_min}/min`));
    if (rowErrors[id]) main.append(el("div", "model-error", rowErrors[id]));

    const actions = el("div", "model-actions");
    if (isActive) {
      actions.append(activePill());
    } else if (p.has_key) {
      actions.append(button("Use", () => act(id, () => invoke("model_activate", { id }))));
    }
    if (p.has_key) {
      actions.append(
        button("Remove key", () => act(id, () => invoke("provider_delete_key", { provider: p.id })), "btn danger-hover")
      );
    } else if (!openKeyForms.has(p.id)) {
      actions.append(
        button("Add key", () => {
          openKeyForms.add(p.id);
          renderOnline();
          document.querySelector(`[data-key-input="${p.id}"]`)?.focus();
        })
      );
    }
    row.append(main, actions);

    if (!p.has_key && openKeyForms.has(p.id)) {
      const form = el("form", "key-form");
      form.style.flexBasis = "100%";
      const input = el("input", "key-input");
      input.type = "password";
      input.placeholder = `${p.name} API key`;
      input.autocomplete = "off";
      input.spellcheck = false;
      input.dataset.keyInput = p.id;
      input.setAttribute("aria-label", `${p.name} API key`);
      const save = el("button", "btn", "Save");
      save.type = "submit";
      const get = button("Get a key", () => invoke("open_url", { url: p.key_url }), "link-btn");
      form.onsubmit = (e) => {
        e.preventDefault();
        save.disabled = true;
        save.textContent = "Checking…";
        act(id, async () => {
          await invoke("provider_save_key", { provider: p.id, key: input.value });
          openKeyForms.delete(p.id);
        });
      };
      form.append(input, save, get);
      row.append(form);
    }
    list.append(row);
  }
}

const FAMILY_NAMES = {
  nemo_transducer: "Parakeet",
  whisper: "Whisper",
  moonshine: "Moonshine",
  qwen3_asr: "Qwen3-ASR",
};

function renderDiscovered() {
  const items = overview.discovered || [];
  $("discovered-block").classList.toggle("hidden", items.length === 0);
  const list = $("discovered");
  list.replaceChildren();
  for (const f of items) {
    const row = el("div", "model-row");
    const main = el("div", "model-main");
    const name = el("div", "model-name", f.title);
    name.append(el("span", "chip plain", "Not rated yet"));
    main.append(name);
    main.append(
      el("div", "model-meta", `${FAMILY_NAMES[f.family] || f.family} family · ${fmtSize(f.size_mb)} · released ${f.released}`)
    );
    row.append(main);
    list.append(row);
  }
}

$("discovered-link").onclick = () => {
  if (overview?.discovery_url) invoke("open_url", { url: overview.discovery_url });
};

function renderModels() {
  if (!overview) return;
  renderLocal();
  renderOnline();
  renderDiscovered();
  const unload = UNLOAD_OPTIONS.find((o) => o.value === settings.unload_after_min);
  $("unload-label").textContent = unload ? unload.name : `After ${settings.unload_after_min} min`;
}

async function loadModels() {
  overview = await invoke("models_overview");
  engineStatus = { status: overview.status };
  for (const m of overview.models) {
    if (!m.downloading) delete downloads[m.id];
  }
  renderModels();
}

createDropdown({
  anchor: $("unload-anchor"),
  trigger: $("unload-trigger"),
  width: 170,
  getItems: () => UNLOAD_OPTIONS.map((o) => ({ ...o, on: o.value === settings.unload_after_min })),
  onPick: (item) => {
    settings.unload_after_min = item.value;
    renderModels();
    saveSettings();
  },
});

// Прогресс скачивания обновляет только полоску, без перерисовки списка.
listen("model-download", (e) => {
  const { id, state, progress, message } = e.payload;
  if (state === "done" || state === "cancelled" || state === "error") {
    delete downloads[id];
    if (state === "error") rowErrors[id] = message || "Download failed";
    loadModels();
    return;
  }
  downloads[id] = { progress, state };
  const box = document.querySelector(`[data-dl="${id}"]`);
  if (!box) return renderModels();
  box.querySelector(".dl-fill").style.width = `${progress}%`;
  const stateText = { verifying: "Checking…", extracting: "Unpacking…" }[state];
  box.querySelector(".dl-pct").textContent = stateText || `${progress}%`;
});
listen("models-changed", () => loadModels());

function setModelStatus(status) {
  engineStatus = { status };
  if (overview) renderModels();
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
    if (tab.dataset.tab === "model") loadModels();
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
