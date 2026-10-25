# Building Voxy on Windows

> **Status:** Windows 10/11 (x64) is a supported platform, but a new one. CI
> (GitHub Actions, `.github/workflows/ci.yml`) builds Voxy on
> `windows-latest`, runs the unit tests there and starts the debug build for a
> short, non-blocking smoke run (the keyboard hook installs, the tray icon and
> island come up). Every release includes the Windows installer. Testing on real hardware
> has been limited so far, so reports from Windows machines are very welcome
> (see [Testing checklist](#testing-checklist)). Background and design
> decisions: [docs/WINDOWS_PORT_PLAN.md](docs/WINDOWS_PORT_PLAN.md).

These steps build Voxy yourself on 64-bit (x64) Windows 10/11, using a
PowerShell prompt. To only install it, use the one-line installer in the
[README](README.md#windows-10-and-11).

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

You don't need to set any environment variables either. Plain `cargo build`
and `cargo test` work as well as `cargo tauri`: on MSVC, `src-tauri/build.rs`
links the C runtime statically by default (`STATIC_VCRUNTIME=true`, which
`cargo tauri` sets too) and embeds the Common Controls v6 manifest
(`src-tauri/windows/app.manifest`) into every binary, unit-test binaries
included.

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

- Installer: `src-tauri\target\release\bundle\nsis\Voxy_<version>_x64-setup.exe`.
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

Unit tests run with `cargo test --lib` (also from `src-tauri`; the tests that
need the network are `#[ignore]`).

To install your build the way users get it, point `install.ps1` at it with
`VOXY_INSTALLER` (from the repository root). It stops a running Voxy, runs the
installer silently and starts the new version:

```powershell
$env:VOXY_INSTALLER = "$PWD\src-tauri\target\release\bundle\nsis\Voxy_<version>_x64-setup.exe"
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
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

- The installer isn't code-signed, so when you run the `.exe` yourself,
  SmartScreen may say "Windows protected your PC". Click **More info → Run
  anyway**. Installing with `install.ps1` (the README's PowerShell command)
  doesn't get this warning. Either way Voxy installs for the current user, in
  `%LOCALAPPDATA%\Voxy`, without administrator rights (Tauri's `currentUser`
  NSIS mode), and the installer adds WebView2 if it is missing.
- Voxy runs in the **notification area** (system tray). On the first launch,
  when no model is installed yet, **the Settings window opens by itself**
  while the default model downloads. Windows 11 may hide new tray icons
  behind the **^** arrow; drag the icon onto the taskbar to keep it visible.
- **Left click** (or double click) the tray icon to open Settings;
  **right click** shows **Settings…** and **Quit Voxy**. Launching Voxy a
  second time (for example from the Start menu) also opens Settings in the
  copy that's already running. Closing Settings leaves Voxy running in the
  tray.
- The default model, Parakeet TDT 0.6B v3 (~670 MB), downloads from Hugging
  Face into `%LOCALAPPDATA%\com.dizraid.voice\models\`. You can follow the
  progress on the **Model** tab in Settings. If you press the hotkey before
  the model is ready, the island shows "Model is not ready yet".
- Everything Voxy stores is under **`%LOCALAPPDATA%\com.dizraid.voice\`**:
  `settings.json`, `history.json`, `discovery.json`, `models\`, the log and
  WebView2's data (`EBWebView\`). Nothing goes to the roaming `%APPDATA%`.
- The log is **`%LOCALAPPDATA%\com.dizraid.voice\Voxy.log`**. Many lines are in
  Russian. `RUST_LOG=debug` adds detail (for example `uipi:` lines from the
  administrator-window check).
- The default hotkey is **Right Ctrl**. To change it, go to **General → Change**
  and press the new key (Esc cancels).
- The hotkey and pasting need **no extra permissions**: Windows has no
  equivalent of macOS's Accessibility and Input Monitoring. The microphone is
  subject to Windows privacy settings, though. If the island shows "Microphone
  unavailable", go to **Settings → Privacy & security → Microphone** (on
  Windows 10: **Settings → Privacy → Microphone**, or run
  `start ms-settings:privacy-microphone`) and allow desktop apps to use the
  microphone.
- API keys for the online providers (OpenAI, Groq, ElevenLabs) are kept in
  Windows **Credential Manager**, as `api-key:<provider>.com.dizraid.voice`
  (`cmdkey /list` shows them).
- **Start with Windows** (General tab) adds a `Voxy` entry under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; it shows up in Task
  Manager → Startup apps.

## Differences from the macOS version

- **Hotkey:** a low-level keyboard hook (`WH_KEYBOARD_LL`) takes the place of
  a CGEventTap. It runs on its own high-priority thread, only one hook is
  installed at a time, and it only listens: the key still reaches the active
  app. Voxy's own synthetic Ctrl+V is ignored, so even Left Ctrl as the hotkey
  doesn't start a recording on every paste; other injected input (PowerToys
  Keyboard Manager, AutoHotkey remaps) still counts. Windows silently removes
  a hook that answers too slowly, for example under heavy load, so Voxy
  reinstalls it every minute (waiting while a key is held, at most 10
  minutes). The log shows `хук установлен` once the hotkey is active, and
  `клавиатурный хук был снят системой, ставлю заново` when Windows had
  removed it.
- **Island:** there's no camera notch, so the pill stays hidden while idle.
  During recording, transcription or an error it rises at the bottom center
  of the primary monitor, just above the taskbar (like the Windows 11 volume
  indicator), and sinks out afterwards; with Windows animation effects turned
  off it only fades. With the taskbar on the side, at the top or
  auto-hidden, it sits at the bottom of the screen. A moved taskbar or a new
  resolution or scale is picked up at the next recording. It's an ordinary
  transparent, always-on-top window (no NSPanel) that never takes the focus
  and appears only once its page has loaded. Clicks pass through it, except
  while recording, when you can click it to get Cancel. Long error messages
  widen the pill to fit (up to 480 px).
- **Settings window:** a native title bar, always dark. Windows 11 (build
  22000 or later) gets a dark Mica backdrop; Windows 10 an opaque dark
  background. The window appears once its page has loaded, so there's no
  blank flash. The browser shortcuts WebView2 would otherwise keep (F5,
  Ctrl+R, Ctrl+P, Ctrl+F, F12 and others) do nothing in Settings and the
  island, and the Chromium context menu appears only in text fields and on
  selected text.
- **Tray:** the colored app icon, readable on light and dark taskbars. Left
  click opens Settings, right click shows the menu (on macOS, a click shows
  the menu).
- **Paste:** a synthetic **Ctrl+V** sent by virtual-key code, so it works with
  any keyboard layout, Cyrillic included. Before sending it, Voxy checks
  whether the active window runs as administrator (see
  [Known gaps](#known-gaps)).
- **Clipboard:** as on macOS, the clipboard's previous text is put back about
  0.5 s after the paste, with the same two exceptions. If you copy something
  in the meantime, Voxy keeps your copy. If the clipboard held a secret from
  a password manager (1Password, Bitwarden and KeePassXC mark them with the
  `ExcludeClipboardContentFromMonitorProcessing` format), it isn't put back,
  as if the manager had cleared it. The dictated text stays out of the Win+V
  clipboard history.
- **Sounds:** Windows system sound aliases, so they follow your sound scheme:
  `SystemAsterisk` (start), `SystemDefault` (stop), `SystemHand` (error). A
  sound that the scheme leaves out (for example with the "No Sounds" scheme)
  stays silent instead of falling back to the default beep.
- **Microphone:** any sample format WASAPI reports (8- to 64-bit integer or
  float). Each recording logs the device, sample rate, channels and format
  (`микрофон: …`).
- **Wording:** the General tab says "Start with Windows" instead of "Launch at
  login", and the hotkey chip pairs a short key cap with the key's name (for
  example "Ctrl" and "Right Ctrl").
- **Log and data:** under `%LOCALAPPDATA%\com.dizraid.voice\` (see
  [First launch](#first-launch)). Saving settings or history retries briefly
  when Windows Defender or the search indexer holds the file.
- **Hotkey names** in `settings.json` are shared between the two systems for
  Ctrl, Shift, Alt, Win (Command on the Mac), Caps Lock, F1–F19 and
  Home/End/Page Up/Page Down/Insert. On Windows, `MetaRight`/`MetaLeft` are
  the Win keys and `AltGr`/`Alt` are Right/Left Alt. `Function` (Fn) exists
  only on macOS. Any other key is saved as a platform-specific code
  (`Key0xNN`), so it doesn't carry over between systems.

## Known gaps

- **Apps running as administrator (UIPI).** Windows keeps a normal-privilege
  process away from elevated windows. Keys typed into an elevated window most
  likely don't reach Voxy's hook, so the hotkey does nothing while one is in
  front. Synthetic input into such a window is blocked too, so Voxy checks
  before it sends Ctrl+V: when the active window is elevated and Voxy isn't,
  the island shows "Paste blocked by an admin app — text kept in clipboard".
  Paste with Ctrl+V yourself; the text is also on the **History** tab.
  Running Voxy itself as administrator avoids the paste block, but isn't
  recommended (`install.ps1` won't start Voxy from an elevated PowerShell).
- **The Win keys and Left Alt are poor hotkeys.** A tap of Win opens the Start
  menu, and a listen-only hook can't stop it; Settings shows a warning when
  you pick one. Left Alt activates app menus (Settings warns about it too),
  and when Voxy brings Settings to the front (for example on a second
  launch), its window library may send a synthetic Left Alt tap, which
  can start or stop a recording in Toggle or Dynamic mode. Right Ctrl or an
  F-key works better.
- **Unsigned installer.** It isn't code-signed, so SmartScreen warns when you
  run the downloaded `.exe` (the PowerShell installer avoids the prompt).
  Code signing is on the roadmap.
- **Clicks above the taskbar while recording.** During a recording the whole
  island window (520×150 at 100% scaling, centered just above the taskbar)
  takes mouse clicks, its transparent parts included. When idle it lets every
  click through.
- **Virtual desktops.** The island may not follow when you switch virtual
  desktops.
- **Primary monitor only.** The island always appears on the primary monitor.
- **No ARM64 build.** ARM PCs run the x64 build under emulation, so
  recognition is slower there.

## Testing checklist

If you try Voxy on Windows, these are the most useful things to check. Please
attach `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` when you report a problem at
https://github.com/DIZRAID/voxy/issues.

**Install and first run**

1. `irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1 | iex`
   on Windows 11 and on Windows 10: it downloads, verifies, installs silently
   and starts Voxy. SmartScreen ("More info → Run anyway") appears only when
   you run the `.exe` by hand.
2. The first launch opens the Settings window: dark title bar, Mica glass on
   Windows 11, opaque on Windows 10, no white flash, centered, not resizable.
   Closing it keeps Voxy in the tray.
3. Tray: the colored icon is visible on dark and light taskbars; left click
   and double click open Settings; right click shows Settings… and Quit Voxy;
   the tooltip reads "Voxy — local dictation".
4. The model download progress shows in Settings → Model; the download goes
   on after you close Settings, and reopening Settings shows its status.

**Hotkey and dictation**

5. Notepad: hold Right Ctrl, speak, release. The text appears at the cursor;
   the island rises at the bottom center, above the taskbar, while recording,
   shows "Transcribing…", then sinks away. The timer changes color near the
   length limit.
6. The same in Word, the Chrome/Edge address bar and a web form, Windows
   Terminal, VS Code, and the Telegram or Slack desktop app.
7. Pasting works while a non-Latin keyboard layout (e.g. Russian) is active.
8. Toggle and Dynamic modes (Recording tab) work; the minimum duration
   ignores short taps; the maximum length stops the recording.
9. Change the hotkey to F8, Caps Lock and Right Alt: each works; Esc cancels
   the capture; the capture ignores keys once Settings loses focus. Left
   Ctrl: a dictation's own Ctrl+V doesn't start a new recording. Right Win
   and Left Alt show a warning under the hotkey, and a tap of Right Win opens
   Start (expected).
10. With Settings focused, F5, Ctrl+P and Ctrl+F don't reload, print or
    search, and a right click shows no browser menu (except in text fields
    and on selected text).
11. Hold the hotkey while a long transcription keeps the CPU busy: the hotkey
    still works afterwards. It also works after 10 idle minutes and after
    sleep and resume, and the log shows no hook errors.
12. Island: click it while recording; Cancel appears and works, and the text
    field in the other app keeps the focus throughout. When idle, the island
    never blocks clicks above the taskbar.

**Clipboard**

13. Copy "A", dictate "B": "B" is pasted, and about 0.5 s later the clipboard
    holds "A" again; Win+V history doesn't list "B".
14. Copy a password from 1Password, Bitwarden or KeePassXC, then dictate: the
    password isn't put back.
15. Copy something during the 0.5 s window (a quick Ctrl+C in another app):
    your copy is kept, not overwritten.
16. Notepad with "Run as administrator": while it is in front, the hotkey
    probably does nothing (expected, see [Known gaps](#known-gaps)). Start a
    dictation in a normal window and click into the elevated Notepad before
    it ends (if Voxy misses the key release, the maximum length stops the
    recording): the island shows "Paste blocked by an admin app — text kept
    in clipboard", Ctrl+V pastes the text, and History has it.

**Settings and system integration**

17. Save, verify and delete an API key (OpenAI, Groq or ElevenLabs);
    `cmdkey /list` shows `api-key:<provider>.com.dizraid.voice`; online
    dictation works.
18. Turn "Start with Windows" on and off and check Task Manager → Startup
    apps; after signing in, Voxy starts in the tray without a window and
    without an island flash.
19. The sounds follow the Windows sound scheme; with the "No Sounds" scheme
    there's silence, not the default beep.
20. The microphone list shows your devices; an unusual microphone (USB mic,
    Bluetooth headset) records; with microphone access turned off in Windows
    privacy settings, the island shows "Microphone unavailable".
21. Several monitors with different scaling (100% and 150%), and the taskbar
    at the top, on the side or auto-hidden: the island sits at the bottom
    center of the primary monitor. Settings text is crisp at 125%, 150% and
    200%.
22. A second launch from the Start menu brings up the running copy's
    Settings; Task Manager shows only one `Voxy.exe`.
23. Update: run the one-liner again while Voxy is running. It is stopped,
    replaced and started again; settings, history and models are kept.
24. Uninstall through Settings → Apps and through `-Uninstall` (see the
    [README](README.md#uninstall-on-windows)). With `-KeepData` the data
    folder stays; without it, `%LOCALAPPDATA%\com.dizraid.voice`, the
    credentials and the Start with Windows entry are gone.
25. The log, `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log`, contains
    `startup complete` and `хук установлен`, and no repeated errors during an
    idle hour.

## NVIDIA GPU acceleration (optional, later)

Recognition currently runs on the CPU. The build uses sherpa-onnx's standard
prebuilt libraries, and `asr.rs` doesn't set `provider`, so sherpa-onnx's
default (CPU) applies. Using CUDA would need three things: a CUDA-enabled build
of sherpa-onnx linked as shared libraries (the crate's `shared` feature in
place of the default `static`, plus `SHERPA_ONNX_LIB_DIR`), shipping its DLLs
with the installer, and setting `provider` in the model config in `asr.rs`.
We'll look at this once the CPU version is stable on Windows.
