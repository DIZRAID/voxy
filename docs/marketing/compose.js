// Voxy marketing compositions (see render.py). Builds the scene for
// ?shot=<name> around the REAL UI:
//  - ui/settings.html's #window, ui/settings.css and ui/settings.js are loaded
//    into this document, so the window's CSS backdrop-filter blurs the
//    wallpaper drawn here (it cannot see through an iframe). settings.js runs
//    in its browser-demo mode (no window.__TAURI__): mock backend, state in
//    window.__settingsDebug. Its built-in .demo-stage (flat blobs) is removed
//    and replaced with the wallpaper below; its traffic lights stay.
//  - ui/island.html is embedded as a transparent iframe (520×150, like the
//    real island window) and driven through contentWindow.__islandDebug.
// Everything is wrapped in an IIFE: settings.js declares many globals and a
// clash with a top-level name here would be a SyntaxError.
(() => {
  "use strict";

  const q = new URLSearchParams(location.search);
  const SHOT = q.get("shot") || "hero";
  const UI = "../../ui/";
  const ICONS = "../../src-tauri/icons/";
  const root = document.documentElement;
  const canvas = document.getElementById("mk-canvas");

  const SIZES = {
    hero: [1440, 960],
    model: [1200, 800],
    general: [1200, 800],
    recording: [1200, 800],
    history: [1200, 800],
    island: [1600, 330],
    "social-preview": [1280, 640],
  };

  // Notch of a 14"/16" MacBook Pro as island.js expects it (its own fallback
  // is 190×34; on notched Macs the menu bar is as tall as the notch).
  const NOTCH = { has_notch: true, notch_w: 190, notch_h: 34, bar_h: 34 };

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // The scene is set on Thursday, Sep 24 2026, 9:41 local time, whatever day
  // it is rendered: the menu bar clock and History's Today/Yesterday agree
  // and re-renders come out the same. Installed before settings.js runs.
  (function sceneClock() {
    const Real = Date;
    const shift = new Real(2026, 8, 24, 9, 41, 0).getTime() - Real.now();
    class SceneDate extends Real {
      constructor(...args) {
        if (args.length) super(...args);
        else super(Real.now() + shift);
      }
      static now() {
        return Real.now() + shift;
      }
    }
    window.Date = SceneDate;
  })();

  function el(tag, cls, attrs) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (attrs) for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    return e;
  }
  function place(e, x, y, w, h) {
    Object.assign(e.style, { position: "absolute", left: `${x}px`, top: `${y}px` });
    if (w != null) e.style.width = `${w}px`;
    if (h != null) e.style.height = `${h}px`;
    return e;
  }

  // icon.png keeps the macOS grid padding: the squircle is 80.5% of the image.
  function appIcon(squircle) {
    const size = squircle / 0.805;
    const img = el("img", "mk-icon", { src: `${ICONS}icon.png`, alt: "" });
    img.style.width = img.style.height = `${size}px`;
    img.style.margin = `${-(size - squircle) / 2}px`;
    return img;
  }

  // ---------------------------------------------------------- wallpaper
  // Large, heavily blurred colour fields from the design palette over the
  // design's #0d0c1c. [colour, centre x %, centre y %, width %, height %, alpha]
  const P = "#ff4f7b"; // pink
  const T = "#1fb6a8"; // teal
  const B = "#4b5bff"; // blue
  const V = "#9d4dff"; // violet
  const WALLS = {
    // settings shots: one broad wash under the window (it tints the glass)
    // plus four fields in the corners; the hue family shifts per shot.
    model: [
      [V, 50, 60, 120, 90, 0.42],
      [P, 8, 16, 62, 72, 0.95],
      [T, 94, 4, 52, 62, 0.85],
      [B, 76, 88, 84, 92, 1],
      [V, 16, 98, 62, 66, 0.95],
    ],
    general: [
      [V, 55, 62, 120, 90, 0.45],
      [P, 14, 20, 74, 78, 1],
      [V, 74, 90, 80, 82, 0.95],
      [T, 97, 6, 42, 50, 0.8],
      [B, 96, 56, 40, 52, 0.7],
    ],
    recording: [
      [B, 50, 60, 120, 90, 0.4],
      [T, 86, 14, 72, 76, 0.95],
      [B, 24, 84, 78, 84, 1],
      [P, 2, 6, 42, 50, 0.8],
      [V, 92, 96, 52, 56, 0.9],
    ],
    history: [
      [B, 50, 56, 120, 90, 0.36],
      [V, 20, 24, 76, 80, 1],
      [P, 90, 86, 62, 68, 0.95],
      [B, 84, 8, 52, 56, 0.85],
      [T, 4, 98, 40, 46, 0.7],
    ],
    // hero/social: colour gathers below and around the window; the top,
    // behind the headline, stays deep so the type reads.
    hero: [
      [V, 50, 84, 110, 60, 0.4],
      [P, 4, 64, 42, 62, 0.95],
      [V, 28, 106, 60, 56, 0.95],
      [B, 74, 98, 68, 68, 1],
      [T, 98, 50, 36, 58, 0.85],
      [V, 50, 16, 76, 34, 0.2],
      [P, 72, 60, 24, 28, 0.3],
    ],
    social: [
      [V, 60, 80, 100, 70, 0.35],
      [P, 2, 92, 50, 74, 0.95],
      [V, 34, 112, 54, 52, 0.8],
      [B, 82, 88, 64, 84, 1],
      [T, 99, 16, 32, 54, 0.8],
      [V, 30, 8, 54, 36, 0.22],
    ],
    island: [
      [V, 22, 118, 60, 80, 0.5],
      [B, 78, 118, 60, 80, 0.5],
      [V, 50, -20, 90, 40, 0.14],
    ],
    // what the two close-up "displays" show below the menu bar: deep at the
    // top (where the island glows), colour further down
    rec: [
      [P, 6, 94, 60, 84, 1],
      [V, 48, 112, 76, 84, 1],
      [B, 94, 92, 60, 84, 1],
      [V, 50, 6, 90, 44, 0.16],
    ],
    proc: [
      [B, 6, 94, 60, 84, 1],
      [V, 50, 114, 76, 84, 0.95],
      [T, 94, 94, 58, 84, 0.95],
      [B, 50, 6, 90, 44, 0.14],
    ],
  };

  // a deep indigo floor under every wallpaper, so there is never a flat
  // black hole between the fields
  const FLOOR = [
    ["#1b1646", 50, 100, 130, 90, 1],
    ["#15122f", 50, 0, 110, 70, 1],
  ];

  // blur: of the colour fields; vig: size of the darkened edge
  function wallpaper(parent, name, blur, vig = blur * 2) {
    const wall = el("div", "mk-wall");
    wall.style.setProperty("--vig", `${vig}px`);
    for (const [c, x, y, w, h, a] of [...FLOOR, ...WALLS[name]]) {
      const f = el("div", "mk-field");
      Object.assign(f.style, {
        left: `${x - w / 2}%`,
        top: `${y - h / 2}%`,
        width: `${w}%`,
        height: `${h}%`,
        background: c,
        opacity: a,
      });
      f.style.setProperty("--blur", `${blur}px`);
      wall.append(f);
    }
    wall.append(el("div", "mk-vignette"));
    parent.append(wall);
    return wall;
  }

  // ---------------------------------------------------- settings window
  function loadCss(href) {
    return new Promise((resolve, reject) => {
      const link = el("link", null, { rel: "stylesheet", href });
      link.onload = resolve;
      link.onerror = () => reject(new Error(`cannot load ${href}`));
      // before compose.css, so the composition's overrides win
      document.head.insertBefore(link, document.getElementById("compose-css"));
    });
  }

  const day = 86400e3;
  function at(daysAgo, h, m) {
    const d = new Date(Date.now() - daysAgo * day);
    d.setHours(h, m, 0, 0);
    return d.getTime();
  }
  // Parakeet TDT 0.6B v3 languages; newest first, as get_history returns them.
  function demoHistory() {
    return [
      { text: "Let's move the design review to Friday at three. I'll send the updated mockups before lunch.", ts_ms: at(0, 9, 24), duration_ms: 6800 },
      { text: "Kannst du mir bis morgen die aktuellen Zahlen für das dritte Quartal schicken?", ts_ms: at(0, 9, 2), duration_ms: 5100 },
      { text: "Perfecto, nos vemos a las ocho en la estación de Atocha.", ts_ms: at(0, 8, 41), duration_ms: 3900 },
      { text: "Merci pour ton retour. Je corrige les deux derniers points et je t'envoie la version finale ce soir.", ts_ms: at(1, 18, 18), duration_ms: 6400 },
      { text: "Созвонимся завтра в десять, я пришлю ссылку на встречу.", ts_ms: at(1, 16, 5), duration_ms: 4200 },
    ];
  }

  async function mountSettings(parent, { tab, x, y, scale }) {
    const [html, js] = await Promise.all([
      fetch(`${UI}settings.html`).then((r) => r.text()),
      fetch(`${UI}settings.js`).then((r) => r.text()),
    ]);
    // what settings.html's inline <head> script does before its stylesheet
    root.dataset.platform = navigator.platform.toUpperCase().indexOf("MAC") >= 0 ? "mac" : "win";
    root.dataset.demo = "";
    root.classList.add("booting");
    await loadCss(`${UI}settings.css`);

    const src = new DOMParser().parseFromString(html, "text/html");
    const holder = place(el("div", "mk-win"), x, y);
    holder.style.transform = `scale(${scale})`;
    holder.append(document.importNode(src.getElementById("window"), true));
    parent.append(holder);

    // Inline, so it runs synchronously right here: createDemo() wraps
    // #window in its .demo-stage and init() starts (its first await is
    // get_settings, so the tweaks below land before anything is rendered).
    const script = el("script");
    script.textContent = `${js}\n//# sourceURL=ui/settings.js`;
    document.body.append(script);

    const win = document.getElementById("window");
    const stage = document.querySelector(".demo-stage");
    if (stage) {
      holder.append(win);
      stage.remove();
    }

    const dbg = window.__settingsDebug;
    if (!dbg) throw new Error("settings demo did not start");
    // The demo's "New releases" row is a made-up future model: keep the
    // discovered list empty so the section stays hidden.
    dbg.state.overview.discovered = [];
    dbg.state.history.splice(0, dbg.state.history.length, ...demoHistory());

    await sleep(120); // init() finishes: settings, status card, permissions
    dbg.tab(tab);
    await sleep(120);
    if (tab === "model") {
      // Qwen3-ASR mid-download (the demo parks it at 42 %)
      const total = 987e6;
      dbg.emit("model-download", {
        id: "qwen3-asr-0.6b",
        state: "downloading",
        progress: 57,
        done_bytes: Math.round(total * 0.57),
        total_bytes: total,
        message: null,
      });
    }
    await sleep(80);
    return dbg;
  }

  // ------------------------------------------------ menu bar and island
  const SVG_SEARCH =
    '<svg width="15" height="15" viewBox="0 0 15 15"><circle cx="6.3" cy="6.3" r="4.9"/><path d="M10 10l3.6 3.6"/></svg>';
  const SVG_WIFI =
    '<svg width="17" height="13" viewBox="0 0 17 13"><path d="M1.3 4.9a10.4 10.4 0 0 1 14.4 0"/><path d="M3.9 7.6a6.6 6.6 0 0 1 9.2 0"/><path d="M6.5 10.2a2.9 2.9 0 0 1 4 0"/></svg>';
  // Control Center: two stacked toggles
  const SVG_CC =
    '<svg width="16" height="15" viewBox="0 0 16 15"><rect x="0.8" y="0.8" width="14.4" height="5.6" rx="2.8"/><circle cx="12.4" cy="3.6" r="1.5" fill="#fff" stroke="none"/><rect x="0.8" y="8.6" width="14.4" height="5.6" rx="2.8"/><circle cx="3.6" cy="11.4" r="1.5" fill="#fff" stroke="none"/></svg>';
  const SVG_BATTERY =
    '<svg width="27" height="13" viewBox="0 0 27 13"><rect x="0.8" y="0.8" width="22.4" height="11.4" rx="3.4" stroke-opacity=".5"/><rect x="2.7" y="2.7" width="15.6" height="7.6" rx="1.7" fill="#fff" stroke="none"/><path d="M25.2 4.8v3.4" stroke-opacity=".5" stroke-width="1.8"/></svg>';

  // items: "full" — a Finder menu bar with Voxy's tray icon among the status
  // items; "short" — the same without the Help menu, for a strip too narrow
  // to leave room between the menus and the recording island; "edges" — only
  // the Apple menu and the clock (close-ups, where the menus would sit under
  // the island anyway).
  function menuBar(items) {
    const bar = el("div", "mk-menubar");
    if (items === "edges") {
      bar.innerHTML =
        '<div class="mk-mb-side"><span class="mk-apple">\uF8FF</span></div>' +
        '<div class="mk-mb-side mk-mb-right"><span class="mk-clock">9:41</span></div>';
    } else {
      const menus = ["File", "Edit", "View", "Go", "Window", ...(items === "short" ? [] : ["Help"])];
      bar.innerHTML =
        `<div class="mk-mb-side"><span class="mk-apple">\uF8FF</span><b>Finder</b>${menus.map((m) => `<span>${m}</span>`).join("")}</div>` +
        `<div class="mk-mb-side mk-mb-right"><img class="mk-tray" src="${ICONS}tray.png" alt="" />${SVG_WIFI}${SVG_BATTERY}${SVG_SEARCH}${SVG_CC}<span class="mk-clock">Thu Sep 24&nbsp;&nbsp;9:41</span></div>`;
    }
    return bar;
  }

  // A strip of "screen" at scale k: menu bar + camera notch, `width` canvas px wide.
  function screenTop(parent, width, k, items) {
    const scr = el("div", "mk-screen");
    scr.style.setProperty("--k", k);
    scr.style.width = `${width / k}px`;
    scr.style.height = "150px";
    scr.style.setProperty("--notch-w", `${NOTCH.notch_w}px`);
    scr.style.setProperty("--notch-h", `${NOTCH.notch_h}px`);
    scr.append(menuBar(items), el("div", "mk-notch"));
    parent.append(scr);
    return scr;
  }

  // clear every pending timer of a frame: the island stops sampling/ticking
  function freeze(w) {
    const last = w.setTimeout(() => {}, 0);
    for (let i = 1; i <= last + 8; i++) w.clearTimeout(i);
  }

  // Recording: run a speech-like level sequence through the island's own
  // sampler (8 bars, envelope with ×0.8 decay, "hot" bars), clocked by us
  // instead of its 110 ms interval so every render gets the same frame.
  async function driveRecording(w, dbg, seconds) {
    // the last 8 become the bars (oldest left): .14 .5 .96 .77 .61 .49 .88 .70
    const levels = [0.1, 0.2, 0.12, 0.05, 0.14, 0.5, 0.96, 0, 0, 0, 0.88, 0];
    let sample = null;
    const realSetInterval = w.setInterval;
    w.setInterval = (fn) => {
      sample = fn; // island.js startSampler(): its per-sample callback
      return 0;
    };
    try {
      dbg.setState("recording", null, 120);
    } finally {
      w.setInterval = realSetInterval;
    }
    if (!sample) throw new Error("island sampler not captured");
    for (const level of levels) {
      dbg.setLevel(level);
      sample();
    }
    // The timer ticks once, at +1 s; shift the frame's clock so that tick
    // reads `seconds`, let it happen, then stop every island timer.
    const now = w.Date.now.bind(w.Date);
    w.Date.now = () => now() + (seconds - 1) * 1000;
    await sleep(1300);
    freeze(w);
    dbg.setLevel(0.95); // the glow breathes with the voice: a loud moment
  }

  async function mountIsland(screen, mode) {
    const frame = el("iframe", "mk-island", { src: `${UI}island.html`, scrolling: "no", tabindex: "-1", "aria-hidden": "true" });
    const loaded = new Promise((r) => frame.addEventListener("load", r, { once: true }));
    screen.append(frame);
    await loaded;
    const w = frame.contentWindow;
    const d = frame.contentDocument;
    const still = d.createElement("style");
    // The timer asks for ui-monospace / "SF Mono": WebKit (the real app)
    // gets SF Mono, Chrome supports neither and falls back to Menlo. render.py
    // serves the system's SF Mono at this private path (never committed); a
    // hand preview without it simply keeps the Menlo fallback.
    still.textContent =
      "@font-face{font-family:'SF Mono';src:url(/__sysfont/SFNSMono.ttf) format('truetype');font-weight:294 900}" +
      "*,*::before,*::after{transition:none!important}" +
      ".proc-label{animation:none!important}" +
      ".spinner{animation:none!important;transform:rotate(40deg)}";
    d.head.append(still);
    const dbg = w.__islandDebug;
    if (!dbg) throw new Error("island demo hook missing");
    dbg.applyMetrics(NOTCH);
    await sleep(30);
    if (mode === "recording") {
      await driveRecording(w, dbg, 7);
    } else {
      dbg.setState(mode);
      freeze(w);
    }
  }

  // --------------------------------------------------------------- shots
  function settingsShot(tab, wall) {
    return async () => {
      wallpaper(canvas, wall, 120);
      // framed tight (the window fills ~78 % of the width), so the UI stays
      // legible in the README's half-width cells; the lowest part of the
      // shadow runs off the bottom edge
      const s = 1.38;
      await mountSettings(canvas, { tab, x: (1200 - 680 * s) / 2, y: (800 - 500 * s) / 2, scale: s });
    };
  }

  const SHOTS = {
    // No brand lockup here: the README's own icon and heading introduce
    // Voxy right above this image. History is the proof of "It types.":
    // dictated text in five languages, and it fits the window uncut.
    async hero() {
      const W = 1440;
      wallpaper(canvas, "hero", 130);
      const s = 1.3; // window and screen strip share one scale: one display
      const settings = mountSettings(canvas, { tab: "history", x: Math.round((W - 680 * s) / 2), y: 256, scale: s });

      const copy = place(el("div", "mk-hero-copy"), 0, 96, W);
      Object.assign(copy.style, { display: "flex", flexDirection: "column", alignItems: "center", gap: "16px" });
      const h1 = el("h1", "mk-h1");
      h1.textContent = "Hold a key. Speak. It types.";
      const sub = el("p", "mk-sub");
      sub.textContent = "Private, on-device dictation for macOS. Works offline in up to 99 languages.";
      copy.append(h1, sub);
      canvas.append(copy);

      await settings;
      const scr = screenTop(canvas, W, s, "short");
      await mountIsland(scr, "recording");
    },

    model: settingsShot("model", "model"),
    general: settingsShot("general", "general"),
    recording: settingsShot("record", "recording"),
    history: settingsShot("history", "history"),

    // Two close-ups of the top of a MacBook display, cropped to the island.
    // No captions in the image: the README's caption under it says it in
    // text. k is near the ceiling for side-by-side panels: the recording
    // pill is ~330 pt wide and the Apple menu and the clock need room.
    async island() {
      const W = 1600;
      const H = 330;
      wallpaper(canvas, "island", 110, 120);
      const devW = 744;
      const gap = 40;
      const x0 = (W - devW * 2 - gap) / 2;
      const top = 40;
      const k = 1.55; // screen points → canvas px inside the close-ups
      const jobs = [];
      [
        ["recording", "rec"],
        ["processing", "proc"],
      ].forEach(([mode, wall], i) => {
        const dev = place(el("div", "mk-device"), x0 + i * (devW + gap), top, devW, H - top);
        const disp = el("div", "mk-display");
        dev.append(disp);
        wallpaper(disp, wall, 50, 50);
        canvas.append(dev);
        const scr = screenTop(disp, devW - 22, k, "edges");
        jobs.push(mountIsland(scr, mode));
      });
      await Promise.all(jobs);
    },

    async "social-preview"() {
      const W = 1280;
      wallpaper(canvas, "social", 100);
      const s = 0.9;
      const settings = mountSettings(canvas, { tab: "model", x: W - 680 * s - 60, y: 128, scale: s });

      // one bold line only: the headline leads, the wordmark is a size down
      // and a weight lighter; everything stays readable in a ~500 px unfurl
      const copy = place(el("div", "mk-social-copy"), 76, 206, 500);
      Object.assign(copy.style, { display: "flex", flexDirection: "column", gap: "20px" });
      const brand = el("div", "mk-brand");
      brand.style.font = "600 44px/1 var(--mk-display)";
      brand.style.gap = "14px";
      brand.append(appIcon(54), Object.assign(el("span"), { textContent: "Voxy" }));
      const h1 = el("h1", "mk-h1");
      h1.style.fontSize = "50px";
      h1.style.marginTop = "12px";
      h1.innerHTML = "Hold a key. Speak.<br>It types.";
      const sub = el("p", "mk-sub");
      sub.style.fontSize = "24px";
      sub.textContent = "Private, on-device dictation for macOS";
      copy.append(brand, h1, sub);
      canvas.append(copy);

      await settings;
      const scr = screenTop(canvas, W, 1, "full");
      await mountIsland(scr, "recording");
    },
  };

  // ---------------------------------------------------------------- run
  (async () => {
    const [w, h] = SIZES[SHOT] || SIZES.hero;
    root.dataset.mk = SHOT;
    root.classList.add("mk-still");
    root.style.setProperty("--mk-w", `${w}px`);
    root.style.setProperty("--mk-h", `${h}px`);
    await (SHOTS[SHOT] || SHOTS.hero)();
    root.dataset.ready = "1";
  })().catch((e) => {
    console.error(e);
    const msg = place(el("pre"), 20, 20);
    Object.assign(msg.style, { color: "#ff6961", font: "14px monospace", zIndex: 99, whiteSpace: "pre-wrap" });
    msg.textContent = `compose.js: ${e && e.stack ? e.stack : e}`;
    document.body.append(msg);
  });
})();
