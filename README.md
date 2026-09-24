[Русский](README.ru.md)

# Voxy

Voxy is push-to-talk dictation for macOS. Hold a key, speak, release: your speech is transcribed and the text is pasted at the cursor in whatever app you are using. Recognition runs on your Mac with open speech models through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). Cloud providers with your own API key are optional. While you speak, a small island at the MacBook camera notch shows a timer and a live waveform.

Status: early development (version 0.1.0). There are no prebuilt releases yet, so you need to [build from source](#build-from-source). A Windows port is written but has not been run on a real Windows machine yet (see [Windows](#windows)).

## Features

- **Dictate into any app.** Hold the hotkey (Right ⌘ by default) and speak. When you release it, the text is pasted at the cursor with a synthetic ⌘V. You can set any single key as the hotkey.
- **Three recording modes.** Hold, Toggle and Dynamic (see [Usage](#usage)).
- **Notch island.** On MacBooks with a camera notch, the idle island matches the notch's measured size. During recording it widens to show a timer, an 8-bar waveform and a glow that follows your voice. On displays without a notch it is hidden until you record, then slides down from the top of the screen.
- **On-device recognition.** The built-in catalog has five local models (see [Models](#models)). If no model is installed, the default one downloads automatically on first launch. Once a model is on disk, dictation works offline.
- **Long dictation.** During a long recording, audio is cut at pauses into pieces of about 20 seconds. Each piece is transcribed while you keep talking, so the wait after release stays short. You can set the maximum length from 15 to 300 seconds (default 120).
- **Model manager.** Download, switch and delete models in Settings → Model. Model revisions are pinned. Interrupted downloads resume, and every download is checked against a SHA-256 hash.
- **Online providers (optional).** Use OpenAI, Groq or ElevenLabs with your own API key.
- **New-model discovery.** At startup, at most once a day, Voxy checks the sherpa-onnx model release on GitHub. It lists up to five new models from families it can run, marked "Not rated yet". These models are only listed, not installed. Models are added to the catalog in app updates, after testing.
- **History.** Voxy keeps up to 50 recent transcriptions, each with a Copy button. You choose how long they are kept: 24 hours, 7 days (default), 30 days or forever.
- **Free memory when idle.** By default the local model is unloaded after 5 minutes without use (options: never, 2, 5, 15 or 30 minutes). Recording still starts as soon as you press the hotkey, and the model reloads while you speak.
- **Other settings:** microphone choice, start/stop sounds, a minimum press length that ignores accidental taps (default 300 ms), and launch at login.
- Voxy runs in the menu bar and has no Dock icon. Only one copy runs at a time: launching Voxy again opens its Settings window.

## Models

Local models, from the built-in catalog (`src-tauri/src/catalog.json`). All are quantized sherpa-onnx exports and run on the CPU.

| Model | Label in the app | On disk | RAM (approx.) | Languages |
|---|---|---|---|---|
| Parakeet TDT 0.6B v3 (NVIDIA), default | Best balance | 670 MB | 1.2 GB | 25 European languages, incl. Russian and Ukrainian; auto-detected |
| Whisper large-v3 turbo (OpenAI) | Most languages · Slower | 1.04 GB | 1.9 GB | 99 languages, auto-detected |
| Qwen3-ASR 0.6B (Alibaba Qwen) | Asian languages | 987 MB | 1.8 GB | 30 languages, incl. Chinese, Japanese, Korean, Arabic, Hindi |
| Parakeet TDT 110M (NVIDIA) | Fastest | 136 MB | 300 MB | English only |
| Moonshine v2 Tiny (Useful Sensors) | Smallest | 44 MB | 100 MB | English only |

For most models the download size equals the size on disk. Parakeet TDT 110M is the exception: it comes as a 108 MB archive and takes 136 MB once extracted.

Online providers (you need your own API key for each):

| Provider | Model | Languages (as listed in the app) |
|---|---|---|
| OpenAI | `gpt-transcribe` | Dozens of languages |
| Groq | `whisper-large-v3-turbo` | 99 languages |
| ElevenLabs | `scribe_v2` | 90+ languages |

The Model tab shows an approximate price per minute for each provider. Billing is between you and the provider.

## Privacy

- **Local models:** audio never leaves your Mac. It is held in memory only while it is recorded and transcribed. Voxy never writes audio to disk.
- **Online providers:** audio goes only to the provider you selected. It is sent as 16 kHz mono WAV, in pieces of up to 28 seconds.
- **API keys** are stored only in the macOS Keychain (Credential Manager on Windows). After you save a key, the app never sends it back to the settings window; it only reports whether a key exists. Before saving, Voxy checks the key with a request that does not use any transcription minutes.
- **History** is plain text in `history.json` on your machine (see [Data locations](#data-locations)). It is kept for the period you choose, and the Clear button deletes it.
- **Clipboard:** pasting works through the clipboard. The dictated text sits there briefly. If the clipboard held text before, that text is put back about 0.5 s later. Clipboard history tools may still record the dictated text.
- **Other network requests:** model downloads (Hugging Face and GitHub) and the once-a-day discovery check (GitHub API). There is no analytics or telemetry.

## Requirements

- macOS 14 Sonoma or later.
- Developed and tested on Apple Silicon. Intel Macs have not been tested.
- Disk space for at least one model (44 MB to 1.04 GB) and enough RAM for it (see [Models](#models)). The default model, which downloads on first launch, takes 670 MB.
- Building from source needs a few GB of free disk space in `src-tauri/target` (see [Build from source](#build-from-source)).
- Windows 10/11 (x64): the port is written but has not been run on real hardware. See [Windows](#windows).

## Build from source

There are no prebuilt releases yet.

Prerequisites (macOS):

1. Xcode Command Line Tools: `xcode-select --install`
2. Rust 1.88 or newer from [rustup.rs](https://rustup.rs). If Rust is already installed, run `rustup update stable`.
3. Tauri CLI v2:
   ```sh
   cargo install tauri-cli --version "^2.0.0" --locked
   ```

Node.js and npm are not needed: the UI in `ui/` is plain HTML, CSS and JavaScript with no build step.

Build:

```sh
git clone https://github.com/DIZRAID/voxy.git
cd voxy/src-tauri
cargo tauri build
```

The app bundle is written to `target/release/bundle/macos/Voxy.app` (inside `src-tauri/`). Still in `src-tauri/`, copy it to `/Applications` and open it:

```sh
cp -R target/release/bundle/macos/Voxy.app /Applications/
open /Applications/Voxy.app
```

**Installing a rebuild.** Quit the running Voxy first (menu bar icon → Quit Voxy). Only one copy of Voxy can run, so while the old one is running, the new build does not start. Then delete the old `/Applications/Voxy.app`, copy the new one and open it. After a rebuild you also have to reset two permissions (see [After every rebuild](#permissions)).

Notes:

- The first build downloads a prebuilt static sherpa-onnx library (with ONNX Runtime, about 20 MB) from the sherpa-onnx GitHub releases. If that download fails, see [Troubleshooting](#troubleshooting).
- Release builds use full LTO, so the final link step takes a while.
- Building needs a few GB of free disk space in `src-tauri/target`. Debug builds from `cargo tauri dev` and `cargo test` take more on top of that. `cargo clean` (run in `src-tauri/`) frees it.
- A build you make on your own Mac opens normally. Builds are not notarized, though, so a copy downloaded from the internet is blocked by Gatekeeper the first time you open it. On macOS 14, Control-click (right-click) the app, choose Open, then confirm. On macOS 15 and later, try to open the app once, then go to System Settings → Privacy & Security and click Open Anyway.

### First launch

1. Voxy's icon appears in the menu bar. Its menu has Settings… and Quit Voxy. If you can't see the icon (on a MacBook with a notch, a full menu bar can hide it), open Voxy again from `/Applications` to bring up Settings.
2. macOS asks for Accessibility and Input Monitoring. The Settings window opens with a banner that shows which permissions are still missing (see [Permissions](#permissions)).
3. If no model is installed yet, the default Parakeet TDT 0.6B v3 (670 MB) starts downloading. Progress appears in Settings → Model. Parakeet covers 25 European languages. For other languages (Qwen3-ASR for Chinese, Japanese, Korean, Arabic or Hindi; Whisper for 99 languages), or for a smaller download, click Cancel on the running download, click Download on the model you want, then click Use when it finishes (downloading a model does not switch to it).
4. macOS asks for microphone access the first time you record. Click Allow, then dictate again.

## Permissions

macOS: System Settings → Privacy & Security.

| Permission | What it is used for |
|---|---|
| Accessibility | Pasting: sends ⌘V to the active app |
| Input Monitoring | The global hotkey. Voxy uses a listen-only event tap, so keystrokes are observed but never blocked or changed |
| Microphone | Recording; macOS asks the first time you record |

The hotkey starts working once both Accessibility and Input Monitoring are granted. You do not need to restart Voxy, because it checks every 2 seconds. The buttons in the Settings banner open the matching System Settings pages.

**After every rebuild.** Builds are ad-hoc signed, so each new build has a different code signature. macOS keeps the old Accessibility and Input Monitoring entries, but they no longer match the new build: the switch still looks on, yet the permission does not work. To fix this, quit the running Voxy (menu bar icon → Quit Voxy). Then go to each of the two lists, select Voxy, remove it with the − button, open the new build and grant the permissions (or add the app back with +). For the same reason, macOS may ask whether Voxy can read a saved API key from the Keychain. Choose Always Allow.

When you run Voxy with `cargo tauri dev`, macOS checks the permissions of the terminal app that started it (Terminal, iTerm and so on). Grant the permissions to that terminal app.

## Usage

1. Put the cursor in any text field.
2. Hold Right ⌘ and speak.
3. Release. The island shows "Transcribing…", then the text is pasted.

| Mode | Behavior |
|---|---|
| Hold (default) | Records while the key is held; releasing it transcribes |
| Toggle | Press once to start, press again to stop |
| Dynamic | A quick tap (under 1 s) starts a recording that continues until the next press. Holding for 1 s or longer works like Hold |

- **Cancel a recording:** click the island while it is recording, then click the Cancel button that appears. The audio is discarded.
- **Accidental taps:** presses shorter than the minimum duration (Settings → Recording, default 300 ms) are ignored.
- **Length limit:** when the maximum length is reached (Settings → Recording, default 120 s), the recording stops and is transcribed. The timer turns yellow 10 seconds before the limit and red 5 seconds before it.
- **Change the hotkey:** Settings → General → Hotkey → Change, then press the new key (Esc cancels). Voxy observes the key without blocking it, so the key still reaches the active app. A modifier or an unused function key works best.
- **Open Settings:** use the menu bar icon → Settings…, or launch Voxy again.
- **History:** Settings → History lists recent transcriptions, each with a Copy button. The Keep menu sets how long entries are kept, and Clear deletes them all.
- **Clipboard:** if the clipboard held text before a paste, that text is restored afterwards. Other content, such as an image, is not restored.

Messages on the island:

| Message | Meaning |
|---|---|
| Model is not ready yet | The active model is not downloaded or failed to load. Check Settings → Model |
| Microphone unavailable | The input device could not be opened |
| Didn't catch that | No speech was recognized |
| Paste failed — text kept in clipboard | The synthetic ⌘V could not be sent, for example because Accessibility was turned off while Voxy was running. Paste with ⌘V yourself; the text is also in History |
| `<Provider>: invalid API key` / `rate limit or no credits` / `No connection to <Provider>` | An error from the online provider |

## Data locations

| What | macOS | Windows |
|---|---|---|
| Settings, history, discovery cache (`settings.json`, `history.json`, `discovery.json`) | `~/Library/Application Support/com.dizraid.voice/` | `%APPDATA%\com.dizraid.voice\` |
| Models | `~/Library/Application Support/com.dizraid.voice/models/` | `%APPDATA%\com.dizraid.voice\models\` |
| Log | `~/Library/Logs/Voxy.log` | `%LOCALAPPDATA%\Voxy.log` |
| API keys | Keychain, service `com.dizraid.voice` | Credential Manager |

The folder is named after the bundle identifier, which is older than the name Voxy.

**Removing Voxy.** First turn off Launch at login in Settings and quit Voxy. Then delete the app, the `com.dizraid.voice` folder and the log file listed above, and the data macOS keeps for the app under the same identifier: `~/Library/WebKit/com.dizraid.voice`, `~/Library/Caches/com.dizraid.voice` and `~/Library/Preferences/com.dizraid.voice.plist`. Remove its Keychain items (service `com.dizraid.voice`) and its entries under Accessibility and Input Monitoring.

## Development

All commands run from `src-tauri/`.

Quit the installed Voxy (menu bar icon → Quit Voxy) before you run `cargo tauri dev`. Only one copy can run: if the installed one is running, the dev build just opens the installed copy's Settings window and exits. Dev and installed builds share the same bundle identifier, so they also share settings, history, downloaded models and API keys.

```sh
cargo tauri dev                          # run the app in development mode
VOXY_LOG_STDERR=1 cargo tauri dev        # same, with logs printed to the terminal
cargo test --lib                         # unit tests
cargo test --lib -- --ignored fake_keys  # network test, see below
```

The ignored `fake_keys` test makes real requests to OpenAI, Groq and ElevenLabs with a fake key. It checks that each provider rejects the key with a clear error.

**End-to-end model check without the UI.** This downloads a catalog model with the same code the app uses (resume, SHA-256, archive extraction), loads it, warms it up and transcribes a WAV file. It prints the timings and the real-time factor:

```sh
cargo run --release --example model_smoke -- <model-id> path/to/audio.wav <models-dir>
```

Model ids: `parakeet-tdt-0.6b-v3`, `whisper-large-v3-turbo`, `qwen3-asr-0.6b`, `parakeet-tdt-110m-en`, `moonshine-v2-tiny-en`. The model is installed into `<models-dir>/<model folder>`.

**Previewing the UI without Tauri.** Outside Tauri, `ui/settings.js` switches to demo mode with mock data. Run this from the repository root:

```sh
python3 -m http.server 4173 --directory ui
```

Then open `http://localhost:4173/settings.html`. For `http://localhost:4173/island.html`, control the island from the browser console:

```js
__islandDebug.setState("recording", null, 120)  // third argument: max length in seconds
__islandDebug.setLevel(0.8)                     // voice level 0..1
__islandDebug.setState("processing")
__islandDebug.setState("error", "Didn't catch that")
__islandDebug.setState("idle")
```

**Adding a model to the catalog.** Add an entry to `src-tauri/src/catalog.json` with a pinned Hugging Face revision (not `resolve/main`) and each file's size and SHA-256, or a `k2-fsa` GitHub archive with its size and SHA-256. The `family` must be one of `nemo_transducer`, `whisper`, `qwen3_asr` or `moonshine`. `cargo test --lib` checks catalog consistency. Then check the new model with `model_smoke`.

## Troubleshooting

**Logs.** Logs are written to `~/Library/Logs/Voxy.log` (on Windows, `%LOCALAPPDATA%\Voxy.log`). Set `VOXY_LOG_STDERR=1` to send them to stderr instead. `RUST_LOG` overrides the default `info` level. Log messages, model download errors shown in Settings → Model, and the `model_smoke` output are currently in Russian.

**The hotkey does nothing.**

- Check the Settings window for the permissions banner: both Accessibility and Input Monitoring must be granted.
- If you have rebuilt the app, quit Voxy, remove the old entries and add Voxy again (see [After every rebuild](#permissions)). After a reinstall, make sure the old copy is not still running.
- If you started Voxy with `cargo tauri dev`, grant the permissions to your terminal app.
- Make sure you are pressing the key shown in Settings → General → Hotkey.
- Check the log. `accessibility=true input_monitoring=true`, followed by a line that contains `event tap установлен`, means the hotkey is active:

  ```sh
  grep -E 'accessibility=|event tap' ~/Library/Logs/Voxy.log | tail
  ```

**Text is recognized but not pasted.** If the island says "Paste failed — text kept in clipboard", the text is in the clipboard: paste it with ⌘V, and check that Accessibility is still granted. Otherwise the ⌘V was sent but the app did not accept it (for example, no text field had focus). The clipboard has already been restored to what it held before, so copy the text from Settings → History.

**Every recording ends with "Didn't catch that".** Check System Settings → Privacy & Security → Microphone: Voxy must be turned on there (with `cargo tauri dev`, your terminal app). Also check the input device in Settings → General → Microphone.

**A model download fails.** The error appears under the model in Settings → Model. Click Download again to resume from where it stopped. If a file fails its checksum, it is deleted and must be downloaded again. Voxy downloads models directly from huggingface.co (Parakeet TDT 110M from github.com) and does not use the macOS proxy settings. If those sites are blocked on your network, use a system-wide VPN.

**The build fails while downloading sherpa-onnx.** The build script of the `sherpa-onnx-sys` crate downloads a prebuilt static library from GitHub. The build honors `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`. If GitHub is still unreachable, download the archive yourself and set `SHERPA_ONNX_ARCHIVE_DIR` to the folder that contains it. The file name must match exactly: `sherpa-onnx-v<version>-osx-arm64-static-lib.tar.bz2` on Apple Silicon, `sherpa-onnx-v<version>-osx-x64-static-lib.tar.bz2` on Intel Macs (untested), or `sherpa-onnx-v<version>-win-x64-static-MT-Release-lib.tar.bz2` on Windows x64. `<version>` is the `sherpa-onnx-sys` version in `src-tauri/Cargo.lock` (1.13.8 at the time of writing). Use an absolute path: the build script does not run in your current directory.

```sh
mkdir -p "$HOME/sherpa-archives"
curl -L -o "$HOME/sherpa-archives/sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2" \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2
SHERPA_ONNX_ARCHIVE_DIR="$HOME/sherpa-archives" cargo tauri build
```

After the first successful download, the archive is cached in `src-tauri/target/sherpa-onnx-prebuilt/`. Alternatively, set `SHERPA_ONNX_LIB_DIR` to the `lib` folder inside an extracted archive (for example `…/sherpa-onnx-v1.13.8-osx-arm64-static-lib/lib`, the folder that holds the `.a` files). It also needs an absolute path.

## Stack

- Rust and [Tauri v2](https://tauri.app): the core, the settings window and the island
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx): local recognition on the CPU (statically linked ONNX Runtime). It uses as many threads as the Mac has performance cores, up to 4
- A custom listen-only `CGEventTap` (macOS) / `WH_KEYBOARD_LL` hook (Windows) for the global hotkey. It can tell the left and right modifier keys apart
- [cpal](https://github.com/RustAudio/cpal) for microphone capture; [enigo](https://github.com/enigo-rs/enigo) and [arboard](https://github.com/1Password/arboard) for pasting
- [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel): a non-activating panel for the island above the menu bar
- [keyring](https://github.com/open-source-cooperative/keyring-rs) for API keys; [ureq](https://github.com/algesten/ureq) for HTTP

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
  settings.html, settings.css, settings.js   settings window
  island.html, island.css, island.js         the island
.github/workflows/  CI (build and unit tests on macOS; build and test compile on Windows) and release builds
README.ru.md        this README in Russian
BUILD_WINDOWS.md    Windows build guide (BUILD_WINDOWS.ru.md in Russian)
```

## Windows

The Windows port is implemented in code but has not been run on a real Windows machine, so expect problems. The CI workflow (`.github/workflows/ci.yml`) builds it and compiles the unit tests on `windows-latest` (they are not run there yet); that checks that the code compiles and links, not that the app works. Platform-specific code is mostly in `platform.rs`, with `#[cfg]` branches in `hotkey.rs`, `island.rs`, `store.rs` (default hotkey) and `lib.rs` (log path, macOS panel setup). The UI detects the platform in `island.js` and `settings.js`. Differences from macOS:

- The default hotkey is Right Ctrl. The global hotkey uses a `WH_KEYBOARD_LL` hook, and no permissions are required.
- Pasting uses a synthetic Ctrl+V, sent by virtual key code so it works with any keyboard layout.
- The island is an ordinary always-on-top window. It is hidden when idle and slides down from the top while recording.
- Sounds are Windows system sounds.
- `cargo tauri build` produces an NSIS installer in `src-tauri\target\release\bundle\nsis\`.

Named keys (modifiers, Caps Lock, F1–F19, Home, End, Page Up, Page Down, Insert) have the same names in `settings.json` on both platforms. Fn and other keys, which are stored as raw key codes (`Key0x…`), do not carry over: on the other platform they map to a different key, or Voxy falls back to the default hotkey.

Build steps are in [BUILD_WINDOWS.md](BUILD_WINDOWS.md) ([in Russian](BUILD_WINDOWS.ru.md)).

## Acknowledgements

Some settings tab icons are adapted from [Feather Icons](https://feathericons.com) (MIT, © Cole Bemis). Speech recognition runs on [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx).
