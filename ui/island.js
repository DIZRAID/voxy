// Вне Tauri (превью) — заглушки + ручное управление через window.__islandDebug.
const IS_TAURI = !!window.__TAURI__;
const { listen } = IS_TAURI ? window.__TAURI__.event : { listen: () => {} };
const invoke = IS_TAURI ? window.__TAURI__.core.invoke : async () => {};

// На Windows нет выреза камеры — в покое пилюлю прячем целиком.
document.documentElement.dataset.platform =
  navigator.platform.toUpperCase().includes("MAC") ? "mac" : "win";

// Геометрия реального выреза из Rust: пилюля в покое рисуется РОВНО по
// физическому вырезу (как Alcove), а не фиксированным размером.
// До прихода метрик пилюля скрыта (:root без data-notch), чтобы не
// мигнуть неправильным размером.
const FALLBACK_METRICS = { has_notch: true, notch_w: 190, notch_h: 34, bar_h: 36 };

function applyMetrics(m) {
  const root = document.documentElement;
  root.style.setProperty("--notch-w", `${m.notch_w}px`);
  root.style.setProperty("--notch-h", `${m.has_notch ? m.notch_h : m.bar_h}px`);
  root.dataset.notch = m.has_notch ? "yes" : "no";
  // страховка от ленивой инвалидации стилей WebKit при смене var на :root
  void document.body.offsetHeight;
}

(IS_TAURI ? invoke("island_metrics") : Promise.resolve(FALLBACK_METRICS))
  .then((m) => applyMetrics(m && typeof m.notch_w === "number" ? m : FALLBACK_METRICS))
  .catch(() => applyMetrics(FALLBACK_METRICS));

const island = document.getElementById("island");
const pill = document.getElementById("pill");
const glowEl = document.getElementById("glow");
const wave = document.getElementById("wave");
const timerEl = document.getElementById("timer");
const errorEl = document.getElementById("error-label");
const cancelChip = document.getElementById("cancel-chip");

// 8 баров: последние 8 сэмплов огибающей, новые справа (спека: 110 мс/сэмпл)
const BARS = 8;
const SAMPLE_MS = 110;
const bars = [];
for (let i = 0; i < BARS; i++) {
  const s = document.createElement("span");
  wave.appendChild(s);
  bars.push(s);
}
const ring = new Array(BARS).fill(0);

let latestLevel = 0; // свежий уровень из Rust (обновляется ~12 Гц)
let envelope = 0;    // сглаженная огибающая (атака быстрая, спад ×0.8)
let sampler = null;
let timerTimeout = null;
let collapseTimeout = null;
let cancelArmTimeout = null;
let processingTimeout = null;
let maxSeconds = 0;

// Если worker распознавания так и не ответил — не висеть в
// «Transcribing…» вечно.
const PROCESSING_SAFETY_MS = 120000;

// Высота полоски 3..16px, полоска нарисована 16px и сжимается scaleY.
function renderBars() {
  for (let i = 0; i < BARS; i++) {
    const level = ring[i];
    bars[i].style.transform = `scaleY(${(3 + level * 13) / 16})`;
    bars[i].classList.toggle("hot", level > 0.55);
  }
}

function startSampler() {
  clearInterval(sampler);
  envelope = 0;
  ring.fill(0);
  renderBars();
  sampler = setInterval(() => {
    envelope = Math.max(latestLevel, envelope * 0.8);
    ring.shift();
    ring.push(Math.min(1, envelope));
    renderBars();
  }, SAMPLE_MS);
}

function stopSampler() {
  clearInterval(sampler);
  sampler = null;
}

// Таймер обновляется ровно раз в секунду, выровненно по границе секунды
// (раньше — 4 раза в секунду при тех же цифрах).
function startTimer() {
  const t0 = Date.now();
  timerEl.textContent = "0:00";
  timerEl.className = "timer";
  clearTimeout(timerTimeout);
  const tick = () => {
    const elapsed = Date.now() - t0;
    const s = Math.floor(elapsed / 1000);
    timerEl.textContent = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
    if (maxSeconds > 0) {
      timerEl.classList.toggle("warn", s >= maxSeconds - 10 && s < maxSeconds - 5);
      timerEl.classList.toggle("danger", s >= maxSeconds - 5);
    }
    timerTimeout = setTimeout(tick, 1000 - (elapsed % 1000) + 5);
  };
  timerTimeout = setTimeout(tick, 1000);
}

function stopTimer() {
  clearTimeout(timerTimeout);
  timerTimeout = null;
}

function disarmCancel() {
  clearTimeout(cancelArmTimeout);
  pill.classList.remove("cancel-armed");
}

function setState(state, message, maxS) {
  clearTimeout(collapseTimeout);
  clearTimeout(processingTimeout);
  disarmCancel();
  if (state === "cancelled") state = "idle";
  island.dataset.state = state;

  if (state === "recording") {
    maxSeconds = maxS || 0;
    startTimer();
    startSampler();
  } else {
    stopTimer();
    stopSampler();
    latestLevel = 0;
    glowEl.style.opacity = "0";
  }
  if (state === "processing") {
    processingTimeout = setTimeout(() => setState("idle"), PROCESSING_SAFETY_MS);
  }
  if (state === "error") {
    errorEl.textContent = message || "Something went wrong";
    collapseTimeout = setTimeout(() => setState("idle"), 2400);
  }
}

// Клик по пилюле во время записи — показать кнопку Cancel на 3 c.
pill.addEventListener("click", () => {
  if (island.dataset.state !== "recording") return;
  if (pill.classList.contains("cancel-armed")) return;
  pill.classList.add("cancel-armed");
  clearTimeout(cancelArmTimeout);
  cancelArmTimeout = setTimeout(disarmCancel, 4000);
});
cancelChip.addEventListener("click", (e) => {
  e.stopPropagation();
  invoke("cancel_recording");
});

listen("state", (e) => setState(e.payload.state, e.payload.message, e.payload.max_s));
listen("level", (e) => {
  if (island.dataset.state !== "recording") return;
  latestLevel = Math.min(1, Math.max(0, Number(e.payload) || 0));
  // Свечение дышит вместе с голосом: меняется только opacity слоя.
  // 0.42 в тишине ≈ прежняя «базовая» яркость ореола.
  glowEl.style.opacity = (0.42 + 0.58 * latestLevel).toFixed(2);
});

// отладка/превью
window.__islandDebug = {
  setState,
  setLevel: (v) => {
    latestLevel = v;
    glowEl.style.opacity = (0.42 + 0.58 * v).toFixed(2);
  },
  applyMetrics,
};
