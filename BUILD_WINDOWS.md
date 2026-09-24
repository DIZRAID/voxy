[Русский](BUILD_WINDOWS.ru.md)

# Building Voxy on Windows

> **Status:** the Windows port is implemented in code but has **not yet been
> verified on real hardware**. So far Voxy has only been built and tested on
> Apple Silicon Macs. CI (GitHub Actions, `.github/workflows/ci.yml`) builds it
> on `windows-latest`. That shows the code compiles, not that the app works.
> Reports from real Windows machines are very welcome (see
> [Testing checklist](#testing-checklist)).

These steps are for 64-bit (x64) Windows 10/11, using a PowerShell prompt.

## Prerequisites (one-time)

1. **Microsoft C++ Build Tools.** Get them from
   https://visualstudio.microsoft.com/visual-cpp-build-tools/ and check the
   **"Desktop development with C++"** workload.
2. **Rust**, via rustup: run the installer from https://rustup.rs, or use
   `winget install --id Rustlang.Rustup`. The toolchain must be **MSVC**
   (sherpa-onnx ships prebuilt libraries only for MSVC):
   ```powershell
   rustup default stable-msvc
   ```
3. **WebView2 Runtime.** Windows 11 and Windows 10 (version 1803 onward)
   already have it. If yours doesn't, get it from
   https://developer.microsoft.com/microsoft-edge/webview2/. The installer you
   build below also downloads and installs WebView2 on machines that lack it
   (this is Tauri's default).
4. **Git**: `winget install --id Git.Git -e` (or https://git-scm.com).
5. **Tauri CLI v2**:
   ```powershell
   cargo install tauri-cli --version "^2.0.0" --locked
   ```

You don't need Node.js or npm, because the UI is plain static files in `ui/`.

The first build needs internet access. Besides the Rust crates that Cargo
fetches, it downloads two more things:
- the sherpa-onnx build script fetches a prebuilt static library archive
  (`sherpa-onnx-v<version>-win-x64-static-MT-Release-lib.tar.bz2`, about
  120 MB) from sherpa-onnx's GitHub releases;
- Tauri fetches the NSIS toolset the first time it builds an installer.

## Getting the source

```powershell
git clone https://github.com/DIZRAID/voxy
cd voxy
```

If the repository is private, Git asks you to sign in to a GitHub account that
has access.

**Alternative: a zip from the Mac.** Leave out `src-tauri/target`, the build
cache, which is rebuilt anyway:

```bash
# on the Mac
cd ~/Desktop && zip -r voice.zip voice -x "voice/src-tauri/target/*"
```

After unzipping on the PC, the folder is named `voice` instead of `voxy`.

## Building

```powershell
cd src-tauri
cargo tauri build
```

The release profile uses LTO and `codegen-units = 1`, so expect the first build
to take a while.

Output:

- Installer: `src-tauri\target\release\bundle\nsis\Voxy_0.1.0_x64-setup.exe`.
  Tauri names it `<productName>_<version>_<arch>-setup.exe`, taking
  `productName` and `version` from `src-tauri/tauri.conf.json`.
- Plain executable: `src-tauri\target\release\voxy.exe`.

`bundle.targets` is `["app", "nsis"]`. `app` is a macOS-only target that Tauri
skips on Windows, so the build produces only the NSIS installer.

For development, `cargo tauri dev` builds a debug version and starts it right
away. To send the log to the console instead of the log file:

```powershell
$env:VOXY_LOG_STDERR = "1"
cargo tauri dev
```

### If the build fails

- **The link step fails and the error points to Git's `link.exe`** (for
  example `...\Git\usr\bin\link.exe`). sherpa-onnx's Windows notes warn that
  Cargo can pick up Git's `link.exe` instead of the MSVC linker. To fix it,
  open **Developer PowerShell for VS** (installed with the Build Tools),
  point Cargo at the MSVC linker, and build again from that window:
  ```powershell
  $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:VCToolsInstallDir 'bin\HostX64\x64\link.exe'
  ```
- **The sherpa-onnx archive can't be downloaded** (you're offline, or a firewall
  blocks it). Download
  `sherpa-onnx-v<version>-win-x64-static-MT-Release-lib.tar.bz2` by hand from
  https://github.com/k2-fsa/sherpa-onnx/releases. `<version>` has to match
  `sherpa-onnx-sys` in `src-tauri/Cargo.lock` (1.13.8 at the time of writing).
  Put the file in a folder, then build with:
  ```powershell
  $env:SHERPA_ONNX_ARCHIVE_DIR = "C:\path\to\that\folder"
  cargo tauri build
  ```
  The build unpacks the archive into `src-tauri\target\sherpa-onnx-prebuilt\`.

## First launch

- The installer isn't code-signed, so SmartScreen may say "Windows protected
  your PC". Click **More info → Run anyway**. The installer installs for the
  current user and doesn't need administrator rights (Tauri's default NSIS
  mode).
- Voxy runs in the **notification area** (system tray) and **opens no window on
  first launch**. Windows 11 may hide new tray icons behind the **^** arrow.
  The tray menu has **Settings…** and **Quit Voxy**. Launching Voxy a second
  time (for example from the Start menu) also opens Settings in the copy
  that's already running.
- The default model, Parakeet TDT 0.6B v3 (~670 MB), downloads automatically
  from Hugging Face into `%APPDATA%\com.dizraid.voice\models\`. You can follow
  the progress on the **Model** tab in Settings. If you press the hotkey
  before the model is ready, the island shows "Model is not ready yet".
- Settings and history are stored in `%APPDATA%\com.dizraid.voice\` (in
  `settings.json` and `history.json`).
- The default hotkey is **Right Ctrl**. To change it, go to **General → Change**
  and press the new key (Esc cancels).
- The hotkey and pasting need **no extra permissions**: Windows has no
  equivalent of macOS's Accessibility and Input Monitoring. The microphone is
  subject to Windows privacy settings, though. If the island shows "Microphone
  unavailable", go to **Settings → Privacy & security → Microphone** (on
  Windows 10: **Settings → Privacy → Microphone**) and allow desktop apps to
  use the microphone.
- API keys for the online providers (OpenAI, Groq, ElevenLabs) are kept in
  Windows **Credential Manager**.

## Differences from the macOS version

- **Hotkey:** a low-level keyboard hook (`WH_KEYBOARD_LL`) takes the place of
  a CGEventTap. Only one hook is installed per process, and it only listens:
  the key still reaches the active app.
- **Island:** there's no camera notch, so the pill stays hidden while idle.
  During recording, transcription or an error it slides down from the top
  edge, centered on the primary monitor. It's an ordinary transparent,
  always-on-top window (no NSPanel). Clicks pass through it, except while
  recording, when you can click it to get Cancel.
- **Paste:** a synthetic **Ctrl+V** sent by virtual-key code, so it works with
  any keyboard layout, Cyrillic included. As on macOS, the clipboard's
  previous text is put back afterwards.
- **Sounds:** Windows system sound aliases, so they follow your sound scheme:
  `SystemAsterisk` (start), `SystemDefault` (stop), `SystemHand` (error).
- **Log:** `%LOCALAPPDATA%\Voxy.log`.
- **Hotkey names** in `settings.json` are shared between the two systems for
  Ctrl, Shift, Alt, Win (Command on the Mac), Caps Lock, F1–F19 and
  Home/End/Page Up/Page Down/Insert. On Windows, `MetaRight`/`MetaLeft` are
  the Win keys and `AltGr`/`Alt` are Right/Left Alt. `Function` (Fn) exists
  only on macOS. Any other key is saved as a platform-specific code
  (`Key0xNN`), so it doesn't carry over between systems.

## Known gaps

These come from reading the code; nobody has seen them on a real machine yet.

- The tray icon is a black glyph drawn as a macOS "template" image, which macOS
  recolors to match the menu bar. Windows shows it as is, so it may be hard to
  see on a dark taskbar.
- Some UI text still says "Mac" (for example "On this Mac" on the Model tab).
- Pasting into apps that run as administrator: Windows (UIPI) blocks
  synthetic input from a normal-privilege process into elevated windows.
  Depending on how Windows reports this, either nothing is pasted, or the
  island shows "Paste failed — text kept in clipboard" and the text stays on
  the clipboard. Either way the recognized text is saved on the **History**
  tab.

## Testing checklist

If you try Voxy on Windows, these are the most useful things to check:

1. The installer runs and the Voxy icon appears in the notification area.
2. The model finishes downloading (Settings → Model).
3. In Notepad: hold Right Ctrl, speak, release. The text appears at the cursor.
4. The island slides down at the top center while recording. Clicking it shows
   Cancel.
5. Pasting works while a non-Latin keyboard layout (e.g. Russian) is active.
6. The Hold / Toggle / Dynamic modes (Recording tab) work, and so does picking
   a different hotkey.
7. Launch at login (General tab) works.
8. Saving an API key for an online provider works.

Please attach `%LOCALAPPDATA%\Voxy.log` when you report a problem at
https://github.com/DIZRAID/voxy/issues.

## NVIDIA GPU acceleration (optional, later)

Recognition currently runs on the CPU. The build uses sherpa-onnx's standard
prebuilt libraries, and `asr.rs` doesn't set `provider`, so sherpa-onnx's
default (CPU) applies. Using CUDA would need three things: a CUDA-enabled build
of sherpa-onnx linked as shared libraries (the crate's `shared` feature in
place of the default `static`, plus `SHERPA_ONNX_LIB_DIR`), shipping its DLLs
with the installer, and setting `provider` in the model config in `asr.rs`.
We'll look at this once the CPU version is stable on Windows.
