# Developing Voxy

[Back to the README](../README.md)

This page is for people who want to run Voxy from source, change it or add a model. If you only want to build and install the app, the [README](../README.md#build-from-source) has everything you need.

- [Stack](#stack)
- [Project layout](#project-layout)
- [Running and testing](#running-and-testing)
- [Checking a model end to end](#checking-a-model-end-to-end)
- [Previewing the UI without Tauri](#previewing-the-ui-without-tauri)
- [Adding a model to the catalog](#adding-a-model-to-the-catalog)
- [Platform-specific code](#platform-specific-code)
- [CI and releases](#ci-and-releases)

## Stack

- Rust and [Tauri v2](https://tauri.app): the core, the Settings window and the island
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx): local recognition on the CPU (statically linked ONNX Runtime). It uses as many threads as the Mac has performance cores, up to 4
- A custom listen-only `CGEventTap` (macOS) / `WH_KEYBOARD_LL` hook (Windows) for the global hotkey. It can tell the left and right modifier keys apart
- [cpal](https://github.com/RustAudio/cpal) for microphone capture; [enigo](https://github.com/enigo-rs/enigo) and [arboard](https://github.com/1Password/arboard) for pasting
- [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel): a non-activating panel for the island above the menu bar
- [tauri-plugin-single-instance](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/single-instance) (only one copy runs; a second launch opens Settings) and [tauri-plugin-autostart](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/autostart) (Launch at login, through a LaunchAgent)
- [keyring](https://github.com/open-source-cooperative/keyring-rs) for API keys; [ureq](https://github.com/algesten/ureq) for HTTP
- The UI in `ui/` is plain HTML, CSS and JavaScript with no build step, so Node.js and npm are not needed

## Project layout

```
src-tauri/
  src/
    main.rs         entry point
    lib.rs          app setup, Tauri commands, menu bar icon, logging
    hotkey.rs       global hotkey and the recording state machine (Hold / Toggle / Dynamic)
    audio.rs        microphone capture, voice level for the island, chunking for long recordings
    worker.rs       recognition thread: loads the active engine, transcribes, pastes, saves history, unloads when idle
    asr.rs          local recognition through sherpa-onnx; splits long audio at pauses
    models.rs       model catalog, downloads with resume, SHA-256 checks, install and delete
    catalog.json    built-in catalog of local models
    online.rs       OpenAI / Groq / ElevenLabs clients; API keys in the system keychain
    discovery.rs    daily check for new models in the sherpa-onnx release
    island.rs       the island window (NSPanel on macOS) and notch measurement
    output.rs       paste through the clipboard and a synthetic ⌘V / Ctrl+V
    platform.rs     platform seam: sounds, permissions, paste keys, inference thread count
    store.rs        settings.json and history.json
  examples/
    model_smoke.rs  end-to-end model check without the UI
  tauri.conf.json   app config (product name, bundle identifier, minimum macOS version)
  Info.plist        microphone usage text; menu-bar-only app (no Dock icon)
ui/
  settings.html, settings.css, settings.js   Settings window
  island.html, island.css, island.js         the island
docs/
  DEVELOPMENT.md    this page
  TROUBLESHOOTING.md
  images/           screenshots used in the README
  marketing/        how those screenshots are rendered
.github/workflows/  CI (build and unit tests on macOS; build and test compile on Windows) and release builds
README.md           the README (README.ru.md in Russian)
BUILD_WINDOWS.md    Windows build guide (BUILD_WINDOWS.ru.md in Russian)
```

## Running and testing

Prerequisites are the same as for [building from source](../README.md#build-from-source). All commands run from `src-tauri/`.

Quit the installed Voxy (menu bar icon → Quit Voxy) before you run `cargo tauri dev`. Only one copy can run: if the installed one is running, the dev build just opens the installed copy's Settings window and exits. Dev and installed builds share the same bundle identifier, so they also share settings, history, downloaded models and API keys.

```sh
cargo tauri dev                          # run the app in development mode
VOXY_LOG_STDERR=1 cargo tauri dev        # same, with logs printed to the terminal
cargo test --lib                         # unit tests
cargo test --lib -- --ignored fake_keys  # network test, see below
```

When you run Voxy with `cargo tauri dev`, macOS checks the permissions of the terminal app that started it (Terminal, iTerm and so on). Grant Accessibility, Input Monitoring and Microphone to that terminal app.

The ignored `fake_keys` test makes real requests to OpenAI, Groq and ElevenLabs with a fake key. It checks that each provider rejects the key with a clear error.

Logging is described in [Troubleshooting](TROUBLESHOOTING.md#logs).

## Checking a model end to end

`model_smoke` downloads a catalog model with the same code the app uses (resume, SHA-256, archive extraction), loads it, warms it up and transcribes a WAV file. It prints the timings and the real-time factor:

```sh
cargo run --release --example model_smoke -- <model-id> path/to/audio.wav <models-dir>
```

Model ids: `parakeet-tdt-0.6b-v3`, `whisper-large-v3-turbo`, `qwen3-asr-0.6b`, `parakeet-tdt-110m-en`, `moonshine-v2-tiny-en`. The model is installed into `<models-dir>/<model folder>`. Its output is currently in Russian.

## Previewing the UI without Tauri

Outside Tauri, `ui/settings.js` switches to a demo mode with mock data that mirrors the real backend: models, providers, downloads, permissions and history. The demo draws the window over a colorful backdrop with painted traffic lights, standing in for the native vibrancy of the real window. Run this from the repository root:

```sh
python3 -m http.server 4173 --directory ui
```

Then open `http://localhost:4173/settings.html`. URL parameters:

| Parameter | Effect |
|---|---|
| `?tab=general`, `record`, `model` or `history` | Opens that tab |
| `?perm=0` | Both permissions missing (shows the permissions banner) |
| `?perm=acc` / `?perm=im` | Only Accessibility / only Input Monitoring missing |

From the browser console, `window.__settingsDebug` drives the demo:

```js
__settingsDebug.tab("model")                   // switch tabs
__settingsDebug.perms(false, true)             // accessibility, input monitoring
__settingsDebug.status("unloaded")             // sidebar card: ready, loading, downloading, unloaded, error, missing
__settingsDebug.download("moonshine-v2-tiny-en")
__settingsDebug.fail("moonshine-v2-tiny-en")   // simulate a failed download
__settingsDebug.addHistory("Hello there", 0)   // text, how long ago in ms
__settingsDebug.state                          // the mock settings, models, history and permissions
```

For the island, open `http://localhost:4173/island.html` and control it from the console:

```js
__islandDebug.setState("recording", null, 120)  // third argument: max length in seconds
__islandDebug.setLevel(0.8)                     // voice level 0..1
__islandDebug.setState("processing")
__islandDebug.setState("error", "Didn't catch that")
__islandDebug.setState("idle")
__islandDebug.applyMetrics({ has_notch: true, notch_w: 190, notch_h: 34, bar_h: 36 })  // notch geometry
```

**README screenshots.** The images in `docs/images/` are rendered from these demo pages with headless Chrome, not captured from the real screen, so the Settings window shows the demo's CSS glass rather than the native material. To re-render them, run `python3 docs/marketing/render.py` (or name specific shots, e.g. `python3 docs/marketing/render.py hero model`) from the repository root; it needs Google Chrome and Pillow. The scenes are built in [`docs/marketing/compose.js`](marketing/compose.js). `social-preview.png` is the repository's social card: upload it under Settings → General → Social preview on GitHub.

## Adding a model to the catalog

Add an entry to [`src-tauri/src/catalog.json`](../src-tauri/src/catalog.json) with a pinned Hugging Face revision (not `resolve/main`) and each file's size and SHA-256, or a `k2-fsa` GitHub archive with its size and SHA-256. The `family` must be one of `nemo_transducer`, `whisper`, `qwen3_asr` or `moonshine`. `cargo test --lib` checks catalog consistency. Then check the new model with [`model_smoke`](#checking-a-model-end-to-end).

## Platform-specific code

Platform-specific code is mostly in `platform.rs`, with `#[cfg]` branches in `hotkey.rs`, `island.rs`, `store.rs` (default hotkey) and `lib.rs` (log path, macOS panel setup). The UI detects the platform in `island.js` and `settings.js`.

Named keys (modifiers, Caps Lock, F1–F19, Home, End, Page Up, Page Down, Insert) have the same names in `settings.json` on both platforms. Fn and other keys, which are stored as raw key codes (`Key0x…`), do not carry over: on the other platform they map to a different key, or Voxy falls back to the default hotkey.

The Windows port has not been run on real hardware yet. See [BUILD_WINDOWS.md](../BUILD_WINDOWS.md) for build steps, the differences from macOS and a testing checklist.

## CI and releases

- [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on pushes to `main` and on pull requests when they change `src-tauri/`, `ui/` or the workflow itself (docs-only changes don't trigger it), and can be started by hand. It makes a debug build and runs the unit tests on macOS (Apple Silicon). On Windows (x64) it builds and compiles the unit tests without running them.
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) runs when you push a tag `v<version>` matching `version` in `src-tauri/tauri.conf.json` (or by hand). It builds a `.dmg` and an `.app` (packed as `.app.tar.gz`) for Apple Silicon and an NSIS installer for Windows x64, and attaches them to a draft GitHub release that you review and publish yourself. Code signing and notarization are not set up yet, so these bundles are unsigned.
