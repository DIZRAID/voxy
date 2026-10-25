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
- [Security notes for contributors](#security-notes-for-contributors)
- [Releases](#releases)

## Build from source

To build on Windows 10/11 (x64), follow [BUILD_WINDOWS.md](../BUILD_WINDOWS.md): it covers the MSVC Build Tools, the Rust toolchain and the NSIS installer. The rest of this section is for macOS.

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

- Rust and [Tauri v2](https://tauri.app): the core, the Settings window and the island. On Windows 11 the Settings window uses Tauri's dark Mica window effect (`MicaDark`); on Windows 10 it is opaque
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx): local recognition on the CPU (statically linked ONNX Runtime). It uses as many threads as the Mac has performance cores (on Windows, half the logical processors), up to 4
- A custom listen-only `CGEventTap` (macOS) / `WH_KEYBOARD_LL` hook (Windows) for the global hotkey. It can tell the left and right modifier keys apart. Both heal themselves: the tap is re-enabled after macOS turns it off, and the Windows hook, which Windows removes without notice when it answers too slowly under heavy load, is reinstalled every 60 seconds while no key is held (a held key delays this by at most 10 minutes). The hook ignores Voxy's own synthetic Ctrl+V
- [cpal](https://github.com/RustAudio/cpal) for microphone capture, in whatever sample format the device offers; [enigo](https://github.com/enigo-rs/enigo) and [arboard](https://github.com/1Password/arboard) for pasting
- System sounds for start, stop and error: `NSSound` on macOS (Pop, Bottle, Basso); on Windows the sound scheme's `SystemAsterisk`, `SystemDefault` and `SystemHand` aliases, silent when the scheme has no sound for one
- [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel) (macOS): a non-activating panel for the island above the menu bar
- [tauri-plugin-single-instance](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/single-instance) (only one copy runs; a second launch opens Settings) and [tauri-plugin-autostart](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/autostart) (Launch at login through a LaunchAgent on macOS; Start with Windows through the `Voxy` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`)
- [keyring](https://github.com/open-source-cooperative/keyring-rs) for API keys (Keychain on macOS, Credential Manager on Windows); [ureq](https://github.com/algesten/ureq) for HTTP
- [windows-sys](https://github.com/microsoft/windows-rs) (Windows) for the Win32 calls: the keyboard hook, sounds, the clipboard sequence number and formats, and the elevation check before a paste; [windows-version](https://github.com/microsoft/windows-rs) for the Windows build number (Mica needs build 22000 or later)
- The UI in `ui/` is plain HTML, CSS and JavaScript with no build step, so Node.js and npm are not needed

## Project layout

```
src-tauri/
  src/
    main.rs         entry point
    lib.rs          app setup, Tauri commands, tray icon, Settings window (show_settings), logging
    hotkey.rs       global hotkey (CGEventTap / WH_KEYBOARD_LL) and the recording state machine (Hold / Toggle / Dynamic)
    audio.rs        microphone capture, voice level for the island, chunking for long recordings
    worker.rs       recognition thread: loads the active engine, transcribes, pastes, saves history, unloads when idle
    asr.rs          local recognition through sherpa-onnx; splits long audio at pauses
    models.rs       model catalog, downloads with resume, SHA-256 checks, install and delete
    catalog.json    built-in catalog of local models
    online.rs       OpenAI / Groq / ElevenLabs clients; API keys in the Keychain / Credential Manager
    discovery.rs    daily check for new models in the sherpa-onnx release
    island.rs       the island window: an NSPanel at the notch (macOS), a transparent always-on-top window bottom-center above the taskbar (Windows)
    output.rs       paste through the clipboard and a synthetic ⌘V / Ctrl+V; puts the old clipboard back unless it changed meanwhile or held a secret
    platform.rs     platform seam: sounds, permissions, paste keys, inference thread count, clipboard generation and secret detection, elevated-window check (Windows)
    store.rs        settings.json and history.json (atomic writes), the data folder, the default hotkey
  examples/
    model_smoke.rs  end-to-end model check without the UI
  windows/
    app.manifest    Common Controls v6 manifest, embedded by link.exe into every MSVC binary (test binaries too)
  build.rs          IPC ACL manifest; for MSVC targets: static C runtime by default and the manifest above
  tauri.conf.json   app config (product name, bundle identifier, minimum macOS version, NSIS installer); the Settings window entry is the macOS one
  Info.plist        microphone usage text; menu-bar-only app (no Dock icon)
  Entitlements.plist  microphone entitlement for signed (hardened runtime) builds
ui/
  settings.html, settings.css, settings.js   Settings window
  island.html, island.css, island.js         the island
  boot.js           sets the platform and backdrop on <html> before the Settings page paints
  guard.js          blocks browser shortcuts (F5, Ctrl+P, Ctrl+F, DevTools) and the browser context menu; loaded by both pages
docs/
  DEVELOPMENT.md    this page
  TROUBLESHOOTING.md
  WINDOWS_PORT_PLAN.md  the plan behind the Windows port: audit, decisions, risks, manual test checklist
  images/           README images and the repository's social card
  marketing/        how those images are rendered
scripts/
  cross-check.sh    type-checks the Windows and macOS code from Linux (see below)
.github/workflows/  CI (build and unit tests on macOS and Windows, a Windows smoke run, an install.ps1 check) and release builds
install.sh          one-command installer for macOS: installs, updates or removes the latest release
install.ps1         one-command installer for Windows (PowerShell): installs, updates or removes the latest release
README.md           the README
BUILD_WINDOWS.md    Windows build guide
CLAUDE.md           notes for coding agents: layout, build and cross-check commands, conventions
```

## Running and testing

Prerequisites are the same as for [building from source](#build-from-source). All commands run from `src-tauri/`.

Quit the installed Voxy (menu bar or tray icon → Quit Voxy) before you run `cargo tauri dev`. Only one copy can run: if the installed one is running, the dev build just opens the installed copy's Settings window and exits. Dev and installed builds share the same bundle identifier, so they also share settings, history, downloaded models, API keys and privacy entries. `install.sh` treats a local build as another copy of Voxy: an install stops it if it is running and resets the permissions it may hold, and `--uninstall` deletes the shared data too (add `--keep-data` to keep it). `install.ps1` on Windows likewise stops a running dev build, and `-Uninstall` deletes the shared data (add `-KeepData` to keep it).

```sh
cargo tauri dev                          # run the app in development mode
VOXY_LOG_STDERR=1 cargo tauri dev        # same, with logs printed to the terminal
cargo test --lib                         # unit tests
cargo test --lib -- --ignored fake_keys  # network test, see below
```

When you run Voxy with `cargo tauri dev`, macOS checks the permissions of the terminal app that started it (Terminal, iTerm and so on). Grant Accessibility, Input Monitoring and Microphone to that terminal app.

The ignored `fake_keys` test makes real requests to OpenAI, Groq and ElevenLabs with a fake key. It checks that each provider rejects the key with a clear error.

Logging is described in [Troubleshooting](TROUBLESHOOTING.md#logs).

### Running and testing on Windows

The same commands work in PowerShell; only the environment variable is set differently:

```powershell
$env:VOXY_LOG_STDERR = "1"; cargo tauri dev   # logs printed to the terminal
cargo test --lib                              # unit tests
```

- **No `STATIC_VCRUNTIME` needed.** For MSVC targets `build.rs` sets it to `true` unless you set it yourself (the sherpa-onnx libraries are built with the static C runtime, `/MT`), so a plain `cargo build` or `cargo test` links the same way as `cargo tauri build`, and the app needs no VC++ Redistributable.
- **The unit tests run on Windows.** `build.rs` has link.exe embed [`src-tauri/windows/app.manifest`](../src-tauri/windows/app.manifest) (Common Controls v6) into every binary, test binaries included. Without it the test binary stopped at load with `0xC0000139` before any test ran: it imports `TaskDialogIndirect` through tao and muda, and only Common Controls v6 has it ([tauri-apps/tauri#13419](https://github.com/tauri-apps/tauri/issues/13419)).
- **`VOXY_LOG_STDERR` needs a debug build.** Release builds are Windows GUI programs with no console, so their stderr goes nowhere; leave the variable unset for them and read the log file.
- **No permissions to grant.** Windows has no equivalent of Accessibility and Input Monitoring; only the microphone is subject to the Windows privacy settings.

## Checking a model end to end

`model_smoke` downloads a catalog model with the same code the app uses (resume, SHA-256, archive extraction), loads it, warms it up and transcribes a WAV file. It prints the timings and the real-time factor:

```sh
cargo run --release --example model_smoke -- <model-id> path/to/audio.wav <models-dir>
```

Model ids: `parakeet-tdt-0.6b-v3`, `whisper-large-v3-turbo`, `qwen3-asr-0.6b`, `parakeet-tdt-110m-en`, `moonshine-v2-tiny-en`. The model is installed into `<models-dir>/<model folder>`. Its output is currently in Russian.

## Previewing the UI without Tauri

Outside Tauri, `ui/settings.js` switches to a demo mode with mock data that mirrors the real backend: models, providers, downloads, permissions and history. The demo draws the window over a colorful backdrop with painted traffic lights, standing in for the native vibrancy of the real window; in the Windows layout it paints a Windows title bar instead. Run this from the repository root:

```sh
python3 -m http.server 4173 --directory ui
```

Then open `http://localhost:4173/settings.html`. URL parameters (combine them with `&`, e.g. `settings.html?platform=win&backdrop=mica`):

| Parameter | Effect |
|---|---|
| `?tab=general`, `record`, `model` or `history` | Opens that tab |
| `?perm=0` | Both permissions missing (shows the permissions banner) |
| `?perm=acc` / `?perm=im` | Only Accessibility / only Input Monitoring missing |
| `?platform=win` / `?platform=mac` | Windows or macOS layout: title bar, key caps, wording such as "Start with Windows". Without it, the preview shows macOS unless the browser runs on Windows |
| `?backdrop=mica` / `?backdrop=opaque` | With the Windows layout: the Windows 11 Mica window or the Windows 10 opaque one (the default) |
| `?hotkey=<key>` | Starts with this hotkey instead of the default, e.g. `MetaRight` (with the Windows layout, shows the warning about the Win key) or `Alt` (Left Alt) |

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

For the island, open `http://localhost:4173/island.html` (add `?platform=win` or `?platform=mac`; the default is the same as for Settings) and control it from the console. In the Windows layout the pill is hidden while idle and rises from the bottom edge, where the taskbar would be:

```js
__islandDebug.setState("recording", null, 120)  // third argument: max length in seconds
__islandDebug.setLevel(0.8)                     // voice level 0..1
__islandDebug.setState("processing")
__islandDebug.setState("error", "Didn't catch that")
__islandDebug.setState("idle")
__islandDebug.applyMetrics({ has_notch: true, notch_w: 190, notch_h: 34, bar_h: 36 })  // notch geometry (macOS layout)
```

**README images.** `docs/images/` holds three images: `hero.png` and `island.png`, which the README shows, and `social-preview.png`, the repository's social card (upload it under Settings → General → Social preview on GitHub). They are rendered from these demo pages with headless Chrome, not captured from the real screen, so the Settings window shows the demo's CSS glass rather than the native material. To re-render all three, run `python3 docs/marketing/render.py` from the repository root (or name specific shots, e.g. `python3 docs/marketing/render.py hero`); it needs Google Chrome and Pillow. The scenes are built in [`docs/marketing/compose.js`](marketing/compose.js). It also has scenes of single Settings tabs (`model`, `general`, `recording`, `history`); the README no longer uses them, and they are rendered only when you name them.

## Adding a model to the catalog

Add an entry to [`src-tauri/src/catalog.json`](../src-tauri/src/catalog.json) with a pinned Hugging Face revision (not `resolve/main`) and each file's size and SHA-256, or a `k2-fsa` GitHub archive with its size and SHA-256. The `family` must be one of `nemo_transducer`, `whisper`, `qwen3_asr` or `moonshine`. `cargo test --lib` checks catalog consistency. Then check the new model with [`model_smoke`](#checking-a-model-end-to-end).

## Platform-specific code

`#[cfg(target_os = …)]` stays at the thin layer that calls the OS; the shared logic is platform-free. The branches are in:

- `platform.rs`, the platform seam: sounds, permissions (always granted on Windows), paste keys, inference thread count, opening URLs, the clipboard generation counter and secret detection, and the elevated-window check before a paste (Windows)
- `hotkey.rs`: the `CGEventTap` listener and the `WH_KEYBOARD_LL` hook thread, each with its key code table
- `island.rs`: the NSPanel at the notch, and the Windows window placed from the primary monitor's work area at creation and at each recording start
- `output.rs`: clipboard writes that stay out of clipboard history (each OS has its own API)
- `store.rs`: the default hotkey (Right Command / Right Ctrl) and the retried file rename on Windows, where Defender or the search indexer can hold a file for a moment
- `lib.rs`: log path, tray icon and click behavior, the macOS panel setup and dark appearance, and the Windows Settings window

**The Settings window.** The `settings` window in `tauri.conf.json` is the macOS one (transparent, overlay title bar, vibrancy). On Windows, `show_settings` in `lib.rs` derives a variant from it at runtime (`windows_settings_config`): always a dark native title bar; on Windows 11 (build 22000 or later, read with `windows-version`) a transparent window with Tauri's `MicaDark` effect, on Windows 10 an opaque `#1a162b` window. It loads `settings.html?backdrop=mica` or `?backdrop=opaque` and is shown only once the page has loaded. Change the macOS window in the config and the Windows one in `windows_settings_config`; unit tests in `lib.rs` check both.

**Testing.** Put decisions in small pure functions without OS calls and keep the OS calls around them thin. Those functions compile into the unit tests on every OS, Windows logic included, so CI tests them on macOS and Windows, and the [cross-check](#cross-checking-windows-and-macos-from-linux) type-checks them from Linux. Examples: `mica_supported` and `windows_settings_config` (`lib.rs`), `bottom_center` (`island.rs`), `uipi_blocks_input` (`platform.rs`), `is_own_input` and `rehook_due` (`hotkey.rs`), `rename_should_retry` (`store.rs`).

**The UI.** `ui/boot.js` runs before the Settings page paints: it sets `<html data-platform="mac|win">` and, from `?backdrop=`, `data-backdrop`; `settings.css` and `settings.js` read those attributes. `ui/island.js` sets `data-platform` for the island the same way. In Tauri the platform comes from `navigator.platform`; in the browser preview from `?platform=`, falling back to the browser's own platform.

Named keys (modifiers, Caps Lock, F1–F19, Home, End, Page Up, Page Down, Insert) have the same names in `settings.json` on both platforms. Fn and other keys, which are stored as raw key codes (`Key0x…`), do not carry over: on the other platform they map to a different key, or Voxy falls back to the default hotkey.

The Windows port is built and unit-tested in CI, and CI starts it for a short smoke run, but it has had little testing on real PCs so far. [BUILD_WINDOWS.md](../BUILD_WINDOWS.md) has the build steps, the differences from macOS and a testing checklist; [WINDOWS_PORT_PLAN.md](WINDOWS_PORT_PLAN.md) records the decisions behind the port, its known risks and a longer manual test checklist.

### Cross-checking Windows and macOS from Linux

Without a Mac or a Windows PC (for example in a cloud session), [`scripts/cross-check.sh`](../scripts/cross-check.sh) type-checks and lints the code for either platform on Linux. Run it from the repository root:

```sh
pip install ziglang                             # once; add --break-system-packages if pip refuses
scripts/cross-check.sh win check --all-targets
scripts/cross-check.sh win clippy --all-targets
scripts/cross-check.sh mac check --all-targets
scripts/cross-check.sh mac clippy --all-targets
```

The script adds the Rust target with rustup, uses zig as the C compiler for the few crates with C code, stubs out `windres` and keeps the sherpa-onnx build script from downloading anything. The output goes to `src-tauri/target/xwin` and `src-tauri/target/xmac` of the main checkout, shared by its git worktrees (`VOXY_CROSS_TARGET_DIR` sets another parent folder). Arguments after the subcommand go to cargo.

What it proves: the `cfg(windows)` and `cfg(target_os = "macos")` code, tests included (`--all-targets`), compiles and passes clippy. What it does not:

- Windows is checked as `x86_64-pc-windows-gnu`, not the MSVC target that CI and releases use. The Rust code is the same, but the MSVC-only part of `build.rs` (the static C runtime default and the embedded manifest) is not exercised. macOS is checked as `aarch64-apple-darwin`.
- Nothing links, so no binary is produced and no test runs (`cargo test --no-run` fails as well). The unit tests run in [CI](#ci) on both systems.
- It says nothing about whether the app works. A native Linux build is not possible either: Tauri would need webkit2gtk, and Linux is not a target.

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on pushes to `main` and on pull requests when they change `src-tauri/`, `ui/`, `install.ps1`, the workflow itself or `.github/scripts/fetch-sherpa-onnx.sh` (docs-only changes don't trigger it), and can be started by hand. On macOS (Apple Silicon) and on Windows (x64) it makes a debug build and runs the unit tests (`cargo test --locked --lib`; the `#[ignore]` network tests are skipped), and the job summary shows the result. `STATIC_VCRUNTIME` is deliberately not set, so the Windows job builds exactly what a plain `cargo build` builds.

Two Windows checks are experimental for now: a failure shows in the run, but it does not fail the workflow (`continue-on-error: true`).

- **Smoke run** (the step "Smoke run (Windows, experimental)"): starts the debug `voxy.exe` for 25 seconds with `VOXY_LOG_STDERR=1`. It passes when the app is still running at the end (exit status 124 from `timeout`) and the log has a line with `хук установлен` (the keyboard hook is installed) and one with `startup complete` (the tray icon and the island exist). It runs whenever the build succeeded, even after failed unit tests, and the job summary quotes the two lines.
- **`installer-script`** (the job "install.ps1 (Windows)", on pushes and manual runs, not on pull requests): checks `install.ps1` under Windows PowerShell 5.1 and PowerShell 7 without installing anything: the file is ASCII-only and parses; `-KeepData` without `-Uninstall` exits with 2; a `VOXY_INSTALLER` file that is not a Windows program is refused with exit code 1 before anything runs; under `iex` a failure leaves the PowerShell session running; the script block form works; and `-Uninstall`, with or without `-KeepData`, succeeds as a no-op on a runner without Voxy.

Once a check has been green on `main`, make it blocking: delete `continue-on-error: true` from the smoke step or from the `installer-script` job, and drop "experimental" from its name and comments.

If the Windows test binary ever stops at load again (`0xC0000139` before any test runs), set `test_args: "--no-run"` in the Windows entry of the build matrix. The step then only builds and links the tests, and the summary says "unit tests compiled, not run".

## Security notes for contributors

- **IPC is scoped per window.** `src-tauri/build.rs` declares an app ACL manifest from the command list in [`src-tauri/src/app_commands.rs`](../src-tauri/src/app_commands.rs), so Tauri checks every `#[tauri::command]` against the window's capability in `src-tauri/capabilities/`. The island window (`island.json`) may call only `island_metrics` and `cancel_recording`; the Settings window (`settings.json`) gets the rest. A new command must be added to `app_commands.rs` and granted as `allow-<command-with-hyphens>` to the window that needs it, or its calls are rejected. The unit test `ipc_is_scoped_per_window` checks all of this. `src-tauri/permissions/autogenerated/` is generated by the build and not committed.
- **Content Security Policy.** `app.security.csp` in `tauri.conf.json` allows only the app's own scripts and styles (no inline `<script>`, no `style="…"` attributes, no remote URLs). Set styles from JavaScript through the CSSOM (`el.style.width = …`), never with `setAttribute("style", …)` or `h(…, { style })`, and keep new scripts in files under `ui/`. Test UI changes in the browser preview with the same policy; the unit test `csp_is_strict` guards the configuration.
- **Native libraries.** The sherpa-onnx static libraries (sherpa-onnx, ONNX Runtime, espeak-ng) come from an upstream GitHub release whose assets can be replaced. CI and the release workflow download them with [`.github/scripts/fetch-sherpa-onnx.sh`](../.github/scripts/fetch-sherpa-onnx.sh), which checks the SHA-256 pinned in [`src-tauri/sherpa-onnx.sha256`](../src-tauri/sherpa-onnx.sha256). When Dependabot or you bump `sherpa-onnx-sys`, add the new hashes there in the same change (see the comment in that file); `cargo test` fails until you do. Local builds download the archive through the crate's build script as before; to build from an archive you checked yourself, set `SHERPA_ONNX_ARCHIVE_DIR` (see [Troubleshooting](TROUBLESHOOTING.md)).

## Releases

1. Set the new version in `src-tauri/tauri.conf.json` (and in `src-tauri/Cargo.toml`), then push a tag `vX.Y.Z` that matches it, e.g. `v0.1.0`. The workflow stops if the tag and `tauri.conf.json` disagree.
2. [`.github/workflows/release.yml`](../.github/workflows/release.yml) checks that the tagged commit is on `main`, creates a draft release, and builds the macOS app for Apple Silicon (`.dmg` and `.app.tar.gz`) and the Windows x64 NSIS installer (`_x64-setup.exe`) in jobs with a read-only token. Both builds are required: the publish job runs only when every build succeeded, so a failed Windows build leaves the release a draft ("Re-run failed jobs" re-runs that build and then publish). The publish step "Check the bundles" checks what the installers will download. On the macOS side, the `.app.tar.gz` and `.dmg` are there, `Voxy.app` sits at the root of the archive with the bundle identifier `com.dizraid.voice`, and its signature verifies, with the hardened runtime and the microphone entitlement. On the Windows side, `Voxy_<version>_x64-setup.exe` is there, is at least 5 MB (a floor kept low until releases show the real size; raise it then) and starts with the `MZ` header of a Windows program. Publish then attests every file (Sigstore build provenance), uploads them into the draft (comparing each upload's SHA-256 with the checked file) and publishes the release. The macOS bundle is signed ad hoc as a whole, with the hardened runtime and the microphone entitlement (`com.apple.security.device.audio-input`). It is not notarized, so Gatekeeper still asks once when the `.dmg` comes from a browser. The Windows installer is not code-signed, so SmartScreen may warn when it is run from a browser download. The release notes, with a section for each system, are written in the workflow. The release is marked Latest right away, so the install commands pick it up immediately. There is no manual trigger: releases come only from `v*` tags. The Tauri CLI (`TAURI_CLI_VERSION`) and the Rust toolchain (`RUST_TOOLCHAIN`) are pinned at the top of the workflow; bump the CLI together with the `tauri` crate. The Windows release build sets `STATIC_VCRUNTIME=true` itself rather than rely on the `build.rs` default.
3. [`install.sh`](../install.sh) always fetches the latest published release through the GitHub API and picks the macOS `.app.tar.gz` by its suffix, so the script does not change between versions. It stops every running copy of Voxy (a local build too), replaces the app and resets the Accessibility and Input Monitoring entries when the new app's code signature requirement differs from the installed one, or when another copy of Voxy may hold them: one with a different signature (for example a local build), or one outside the Applications folders, which the script does not open. With ad-hoc signing the signature differs on every release; a stable signing identity would keep the permissions across updates. `--reset-permissions` resets the two entries and restarts Voxy without downloading anything.
4. [`install.ps1`](../install.ps1) does the same on Windows. It fetches the latest published release through the GitHub API (its tag must look like `vX.Y.Z`), picks the asset whose name ends in `_x64-setup.exe`, checks its SHA-256 against the digest GitHub lists for it (it warns when there is none), stops the running Voxy (a local build too), runs the installer silently (`/S`: per user into `%LOCALAPPDATA%\Voxy`, no administrator rights) and starts Voxy. From a PowerShell that runs as administrator it installs but does not start Voxy, which would then run elevated too. It warns on ARM64 PCs (x64 emulation) and on Windows older than 1809. Options: `-NoLaunch` (install or update without starting Voxy), `-Uninstall` (run Voxy's uninstaller, remove the "Start with Windows" entry, the data folder and the API keys) and `-Uninstall -KeepData` (keep the models, settings, history and API keys). `iex` cannot pass options, so they need the script block form: `& ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -NoLaunch`. The script must stay ASCII-only (Windows PowerShell 5.1 reads a script without a BOM in the ANSI code page) and keeps CRLF line endings (`.gitattributes`).

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

`install.ps1` reads these environment variables:

| Variable | Effect |
|---|---|
| `VOXY_INSTALLER` | Install this local `-setup.exe` instead of downloading the latest release (it must still be a Windows program, or the script stops before running it) |
| `GITHUB_TOKEN` | Sent to api.github.com only: a higher rate limit, or access to a private repository. If GitHub rejects it (HTTP 401), the script goes on without it |
| `NO_COLOR` | No colored output |

Run as a file (`powershell -File install.ps1`, `& .\install.ps1`), it exits with `0` when done, `1` when something failed or the system is not supported, and `2` for options that don't go together (`-KeepData` without `-Uninstall`, `-NoLaunch` with `-Uninstall`). Through `iex` or a script block it never calls `exit`, which would close the PowerShell window; the code goes to `$LASTEXITCODE` instead.

Unlike `install.sh`, `install.ps1` has no way to install somewhere else or to use other data folders. Even with `VOXY_INSTALLER` it really installs Voxy for your user (into `%LOCALAPPDATA%\Voxy`, replacing an installed copy and stopping a running one), and `-Uninstall` really deletes `%LOCALAPPDATA%\com.dizraid.voice` and the API keys. To keep your own setup untouched, test in a spare Windows account, a virtual machine or Windows Sandbox. For example, to install an installer you built with `cargo tauri build` without starting it, then remove it again but keep the data (from the repository root):

```powershell
$env:VOXY_INSTALLER = "C:\path\to\Voxy_<version>_x64-setup.exe"
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -NoLaunch
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Uninstall -KeepData
Remove-Item Env:VOXY_INSTALLER
```
