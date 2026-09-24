# Developing Voxy

[Back to the README](../README.md)

This page is for people who want to build Voxy from source, change it or add a model. If you only want to use the app, the [README](../README.md#install) installs a prebuilt release with one command.

- [Build from source](#build-from-source)
- [Stack](#stack)
- [Project layout](#project-layout)
- [Running and testing](#running-and-testing)
- [Checking a model end to end](#checking-a-model-end-to-end)
- [Previewing the UI without Tauri](#previewing-the-ui-without-tauri)
- [Adding a model to the catalog](#adding-a-model-to-the-catalog)
- [Platform-specific code](#platform-specific-code)
- [CI](#ci)
- [Releases](#releases)

## Build from source

You need macOS 14 or later and a few GB of free disk space in `src-tauri/target`. Voxy is developed and tested on Apple Silicon; building on an Intel Mac has not been tested. Install three things:

1. Xcode Command Line Tools: `xcode-select --install`
2. Rust 1.88 or newer from [rustup.rs](https://rustup.rs). If Rust is already installed, run `rustup update stable`.
3. Tauri CLI v2:
   ```sh
   cargo install tauri-cli --version "^2.0.0" --locked
   ```

Node.js and npm are not needed: the UI in `ui/` is plain HTML, CSS and JavaScript with no build step.

Build Voxy and copy it to `/Applications`:

```sh
git clone https://github.com/DIZRAID/voxy.git
cd voxy/src-tauri
cargo tauri build
cp -R target/release/bundle/macos/Voxy.app /Applications/
open /Applications/Voxy.app
```

Good to know:

- The first build downloads a prebuilt static sherpa-onnx library (with ONNX Runtime, about 20 MB) from the sherpa-onnx GitHub releases. If that download fails, see [Troubleshooting](TROUBLESHOOTING.md#the-build-fails-while-downloading-sherpa-onnx).
- Release builds use full LTO, so the final link step takes a while.
- Debug builds from `cargo tauri dev` and `cargo test` take more disk space on top of the release build. `cargo clean` (run in `src-tauri/`) frees it.
- **Gatekeeper.** A build you make on your own Mac opens normally. A plain `cargo tauri build` does not sign the bundle as a whole (only the linker signs the executable), so a copy you send to someone else opens as "damaged" on their Mac, with no Open Anyway. Share a release instead, or build with `APPLE_SIGNING_IDENTITY=-` as [`release.yml`](../.github/workflows/release.yml) does.

### Installing a new build

Quit the running Voxy first (menu bar icon → Quit Voxy). Only one copy of Voxy can run, so while the old one is running, the new build does not start. Delete the old `/Applications/Voxy.app`, copy the new one and open it.

Local builds are ad-hoc signed, so every build has a different code signature and the old Accessibility and Input Monitoring entries stop working. Reset them with the two `tccutil` commands in [After an update](../README.md#after-an-update), then grant the permissions again.

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
  Entitlements.plist  microphone entitlement for signed (hardened runtime) builds
ui/
  settings.html, settings.css, settings.js   Settings window
  island.html, island.css, island.js         the island
docs/
  DEVELOPMENT.md    this page
  TROUBLESHOOTING.md
  images/           README images and the repository's social card
  marketing/        how those images are rendered
.github/workflows/  CI (build and unit tests on macOS; build and test compile on Windows) and release builds
install.sh          one-command installer: installs, updates or removes the latest release
README.md           the README
BUILD_WINDOWS.md    Windows build guide
```

## Running and testing

Prerequisites are the same as for [building from source](#build-from-source). All commands run from `src-tauri/`.

Quit the installed Voxy (menu bar icon → Quit Voxy) before you run `cargo tauri dev`. Only one copy can run: if the installed one is running, the dev build just opens the installed copy's Settings window and exits. Dev and installed builds share the same bundle identifier, so they also share settings, history, downloaded models, API keys and privacy entries. `install.sh` treats a local build as another copy of Voxy: an install stops it if it is running and resets the permissions it may hold, and `--uninstall` deletes the shared data too (add `--keep-data` to keep it).

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

**README images.** `docs/images/` holds three images: `hero.png` and `island.png`, which the README shows, and `social-preview.png`, the repository's social card (upload it under Settings → General → Social preview on GitHub). They are rendered from these demo pages with headless Chrome, not captured from the real screen, so the Settings window shows the demo's CSS glass rather than the native material. To re-render all three, run `python3 docs/marketing/render.py` from the repository root (or name specific shots, e.g. `python3 docs/marketing/render.py hero`); it needs Google Chrome and Pillow. The scenes are built in [`docs/marketing/compose.js`](marketing/compose.js). It also has scenes of single Settings tabs (`model`, `general`, `recording`, `history`); the README no longer uses them, and they are rendered only when you name them.

## Adding a model to the catalog

Add an entry to [`src-tauri/src/catalog.json`](../src-tauri/src/catalog.json) with a pinned Hugging Face revision (not `resolve/main`) and each file's size and SHA-256, or a `k2-fsa` GitHub archive with its size and SHA-256. The `family` must be one of `nemo_transducer`, `whisper`, `qwen3_asr` or `moonshine`. `cargo test --lib` checks catalog consistency. Then check the new model with [`model_smoke`](#checking-a-model-end-to-end).

## Platform-specific code

Platform-specific code is mostly in `platform.rs`, with `#[cfg]` branches in `hotkey.rs`, `island.rs`, `store.rs` (default hotkey) and `lib.rs` (log path, macOS panel setup). The UI detects the platform in `island.js` and `settings.js`.

Named keys (modifiers, Caps Lock, F1–F19, Home, End, Page Up, Page Down, Insert) have the same names in `settings.json` on both platforms. Fn and other keys, which are stored as raw key codes (`Key0x…`), do not carry over: on the other platform they map to a different key, or Voxy falls back to the default hotkey.

The Windows port has not been run on real hardware yet. See [BUILD_WINDOWS.md](../BUILD_WINDOWS.md) for build steps, the differences from macOS and a testing checklist.

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on pushes to `main` and on pull requests when they change `src-tauri/`, `ui/` or the workflow itself (docs-only changes don't trigger it), and can be started by hand. It makes a debug build and runs the unit tests on macOS (Apple Silicon). On Windows (x64) it builds and compiles the unit tests without running them.

## Releases

1. Set the new version in `src-tauri/tauri.conf.json` (and in `src-tauri/Cargo.toml`), then push a tag `vX.Y.Z` that matches it, e.g. `v0.1.0`. The workflow stops if the tag and `tauri.conf.json` disagree.
2. [`.github/workflows/release.yml`](../.github/workflows/release.yml) creates a draft release, builds the macOS app for Apple Silicon (`.dmg` and `.app.tar.gz`) and the Windows x64 NSIS installer (`-setup.exe`) into it, checks the uploaded macOS app (signature, hardened runtime, microphone entitlement) and publishes the release. The macOS bundle is signed ad hoc as a whole, with the hardened runtime and the microphone entitlement (`com.apple.security.device.audio-input`). It is not notarized, so Gatekeeper still asks once when the `.dmg` comes from a browser. The release is marked Latest right away, so the install command picks it up immediately. The workflow can also be run by hand (Actions → Release → Run workflow); it then releases the version in `tauri.conf.json`.
3. [`install.sh`](../install.sh) always fetches the latest published release through the GitHub API and picks the macOS `.app.tar.gz` by its suffix, so the script does not change between versions. It stops every running copy of Voxy (a local build too), replaces the app and resets the Accessibility and Input Monitoring entries when the new app's code signature requirement differs from the installed one, or when another copy of Voxy may hold them: one with a different signature (for example a local build), or one outside the Applications folders, which the script does not open. With ad-hoc signing the signature differs on every release; a stable signing identity would keep the permissions across updates. `--reset-permissions` resets the two entries and restarts Voxy without downloading anything.

`install.sh` reads a few environment variables, so you can test it without touching your real installation:

| Variable | Effect |
|---|---|
| `VOXY_ARCHIVE` | Install from this local `.app.tar.gz` instead of downloading the latest release; a bad code signature is then only a warning |
| `VOXY_APP_DIR` | Use this folder instead of `/Applications` |
| `VOXY_NO_TCC` | Don't reset privacy permissions |
| `VOXY_NO_LAUNCH` | Don't open Voxy |
| `VOXY_NO_KILL` | Don't stop a running Voxy |
| `VOXY_DATA_ROOT` | A folder that stands in for your home folder (Voxy's data, the login item, the Trash); the Keychain and `launchctl` are skipped |

For example, to install a release archive you already downloaded into a throwaway folder:

```sh
VOXY_ARCHIVE=path/to/Voxy.app.tar.gz VOXY_APP_DIR="$(mktemp -d)" VOXY_DATA_ROOT="$(mktemp -d)" \
  VOXY_NO_TCC=1 VOXY_NO_LAUNCH=1 VOXY_NO_KILL=1 bash install.sh
```
