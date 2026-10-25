# Voxy Windows Port — Implementation Plan

Paths in this document are relative to the repository root (`/home/user/voxy`). Line numbers refer to the tree at commit `87c9ca3` (Release 0.1.1).

## 0. Summary and headline decisions

The Windows code path compiles but has never run. Reading it against Win32, Tauri 2.11.5, tao 0.35.3, enigo 0.6.1, arboard 3.6.1, keyring 4.2.0 and cpal 0.16.0 (all sources verified locally in `~/.cargo/registry`) turns up no fatal bug, but a dozen robustness gaps plus the known cosmetic ones. The plan fixes them in five parallel workstreams with disjoint file ownership, after a small sequential Phase 0 that freezes the shared hot-spot files (`Cargo.toml`, `Cargo.lock`, `tauri.conf.json`).

Decisions (justified in the audit and briefs):

| Area | Decision |
|---|---|
| Settings window chrome | Keep the **native title bar**, force the window **theme Dark** (per-window on Windows, unlike macOS). **Mica (dark)** backdrop on Windows 11 (build ≥ 22000) with a transparent window; **opaque** fallback on Windows 10. No custom title bar (unverifiable without hardware). |
| Tray | Colored 32×32 app icon on Windows (works on light and dark taskbars); **left click / double click opens Settings**, menu on right click (`show_menu_on_left_click(false)`). |
| First run | Settings window opens at startup when the default model must be downloaded (today it only opens for missing macOS permissions, so Windows users see nothing). |
| Data locations | Everything under **`%LOCALAPPDATA%\com.dizraid.voice\`** (settings, history, models, discovery cache, `Voxy.log`) via `app_local_data_dir()`; identical on macOS, so no migration. Nothing has shipped on Windows, so no migration there either. |
| Hotkey hook | `GetModuleHandleW` for `hMod`, named hook thread at `THREAD_PRIORITY_HIGHEST`, ignore Voxy's own injected input (enigo marker), 60 s self-healing re-hook while no key is held, logging only outside the hook callback. |
| Clipboard | Real `clipboard_generation()` via `GetClipboardSequenceNumber`; `clipboard_holds_secret()` via the `ExcludeClipboardContentFromMonitorProcessing` format (set by 1Password/Bitwarden/KeePassXC). |
| Paste into elevated apps (UIPI) | Cannot be fixed from a user-level process; detect it (`TokenElevation` of the foreground window's process) and show a specific island message plus a log line. |
| Build/CI | Embed the Common Controls v6 manifest into **test** binaries (`cargo:rustc-link-arg-tests`), run Windows unit tests in CI, add an experimental 25 s smoke run of `voxy.exe`, default `STATIC_VCRUNTIME=true` on MSVC. |
| Release | Windows becomes a required artifact; the release stays a draft if the installer is missing. New one-command installer `install.ps1` (`irm … | iex`). |
| Out of scope | Code signing, auto-updater, ARM64 Windows, GPU inference, custom title bar, winget, Windows 7/8, light theme. |

---

## 1. Audit findings

Severity: **H** = functional bug / data risk, **M** = robustness or UX bug, **L** = polish.

### 1.1 `src-tauri/src/hotkey.rs` (Windows listener, lines 685–764)

| # | Sev | Where | Finding |
|---|---|---|---|
| A1 | M | L754 `SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), std::ptr::null_mut(), 0)` | `hMod` is null. The documented contract for `SetWindowsHookEx` with `dwThreadId = 0` requires a module handle; low-level hooks happen to work with NULL in practice because they run in the installing process, but we are relying on undocumented leniency. Pass `GetModuleHandleW(null)`. |
| A2 | H | L729–751 `hook_proc` | No filtering of **Voxy's own synthetic input**. `output::send_paste` (enigo) injects Ctrl-down, V, Ctrl-up with `KBDLLHOOKSTRUCT.dwExtraInfo == enigo::EVENT_MARKER` (= 100, verified in `enigo-0.6.1/src/lib.rs:87` and `win/win_impl.rs:395`). If the user picks `ControlLeft` (or any key enigo sends) as hotkey, every paste re-enters the recording state machine (PTT: starts a new recording; Toggle: toggles). Default `ControlRight` escapes only by luck (enigo sends the generic `VK_CONTROL`, scan 0x1D = left). Fix: `if kb.dwExtraInfo == enigo::EVENT_MARKER as usize { return CallNextHookEx(...) }`. |
| A3 | M | L753–763 hook thread | No thread priority. `WH_KEYBOARD_LL` callbacks must return within `LowLevelHooksTimeout` (default 300 ms on Win10/11) or Windows **silently removes the hook**. The callback itself is fast (atomics + unbounded channel), but it only runs when the hook thread gets scheduled; during a 4-thread sherpa-onnx inference on a 4-core PC a normal-priority thread can be starved past the timeout. No recovery exists once the hook is dropped. Fix: `SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST)` plus a periodic re-hook (see A4). |
| A4 | M | L761–762 `while GetMessageW(...) > 0 {}` | No self-healing (macOS re-enables the tap every 30 s and in the callback). After a silent removal the hotkey is dead until restart with nothing in the log. Fix: `SetTimer(NULL, 0, 60_000, None)` on the hook thread; on `WM_TIMER` (when the `down` mask is all zero, so no release is lost) `UnhookWindowsHookEx` + `SetWindowsHookExW` again, logging failures. Also log when `GetMessageW` returns ≤ 0 (today the thread just exits). |
| A5 | M | L157–158 (`MetaRight`/`MetaLeft` = Win keys) | A **tap of the Win key opens the Start menu** on Windows, so Win is unusable as a hotkey, yet it is offered and a `settings.json` copied from a Mac (`MetaRight`) maps to Right Win. Cannot be suppressed by a listen-only hook. Mitigation is UI-side (WS-D: warning under the hotkey row) and docs. |
| A6 | L | L1–13 module doc, L685–687 | Docs describe only the macOS tap. Document the Windows invariants (no logging/locks in `hook_proc`, timeout, injected-input filter, Win/Alt caveats). |
| A7 | L | L753 `std::thread::spawn` | Unnamed thread; `GetLastError` not logged on failure. |

Also noted (no change): `tao::force_window_active` (`tao-0.35.3/src/platform_impl/windows/window.rs:1500–1527`) injects a synthetic **Left Alt tap** (`dwExtraInfo` 0, `LLKHF_INJECTED`) when `SetForegroundWindow` is refused, e.g. when the second instance asks the first to show Settings. With `Alt` (Left Alt) as hotkey in Toggle mode this would toggle recording. Filtering all `LLKHF_INJECTED` events would also break users of PowerToys Keyboard Manager / AutoHotkey remaps, so the plan filters only Voxy's own marker and documents that Left Alt is a poor hotkey on Windows.

### 1.2 `src-tauri/src/island.rs` (Windows `create`, lines 151–176)

| # | Sev | Where | Finding |
|---|---|---|---|
| I1 | M | L155–165 | Window is created visible (`visible` default true) before the page loads: WebView2 paints its default background first, so a 520×150 rectangle can flash at the top of the screen at every launch (including autostart at login). Fix: `.visible(false)` + `.on_page_load(|w, p| if p.event() == PageLoadEvent::Finished { w.show() })`. |
| I2 | M | L155–165 | No `.focused(false)`. tao shows the window with `SW_SHOW` (activating) unless `MARKER_DONT_FOCUS` is set (`window_state.rs:329–333`); `focusable(false)` adds `WS_EX_NOACTIVATE`, but `.focused(false)` is the explicit, documented way to get `SW_SHOWNOACTIVATE`. Without it the island may steal foreground from whatever the user is typing in when Voxy starts. |
| I3 | L | L167–174 | Position ignores `monitor.position()` (fine for the primary monitor, whose origin is always 0,0) and the **work area**: with a top-docked taskbar the island overlaps it. Use `monitor.work_area()` (exists in tauri 2.11.5, `window/mod.rs:96`) and physical coordinates. |
| I4 | L | L164 `.visible_on_all_workspaces(true)` | No-op on Windows (tao has no implementation). Harmless; keep with a comment or drop. |

### 1.3 `src-tauri/src/output.rs`, `src-tauri/src/platform.rs`, `src-tauri/src/worker.rs`, `src-tauri/src/audio.rs`

| # | Sev | Where | Finding |
|---|---|---|---|
| O1 | H | `platform.rs:226–229` (`clipboard_generation` → `None`), used at `output.rs:29,41` | On Windows the "did someone else write the clipboard meanwhile" check is disabled, so the 500 ms restore at `output.rs:41–42` **overwrites anything the user copied during the paste window**. `GetClipboardSequenceNumber()` (Win32_System_DataExchange) is the exact equivalent of `NSPasteboard.changeCount`. |
| O2 | M | `platform.rs:250–253` (`clipboard_holds_secret` → `false`) | Password-manager secrets are read and put back. Windows managers mark secrets with the `ExcludeClipboardContentFromMonitorProcessing` format; arboard's `exclude_from_history()` sets only `CanIncludeInClipboardHistory` (verified `arboard-3.6.1/src/platform/windows.rs:800–845`), so checking that format does not misfire on Voxy's own writes. |
| O3 | M | `platform.rs:45–57` `play` | `wide` (the alias string) is dropped as soon as `PlaySoundW(..., SND_ALIAS | SND_ASYNC)` returns. The API resolves aliases synchronously in practice, but the contract does not promise it. Use static strings (`windows_sys::w!("SystemAsterisk")`) and add `SND_NODEFAULT` so a missing alias stays silent instead of beeping. |
| O4 | M | `output.rs:70–81`, `worker.rs:323–334` | Pasting into an **elevated** window: `SendInput` is blocked by UIPI and, per the docs, "neither GetLastError nor the return value will indicate the failure was caused by UIPI". enigo returns `InputError::Simulate` → generic "Paste failed — text kept in clipboard" with no clue. Detect elevation of the foreground process (`GetForegroundWindow` → `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken(TOKEN_QUERY)` → `GetTokenInformation(TokenElevation)`), log it and show a specific message. |
| O5 | L | `output.rs:71–72` | `Enigo::new` error text says "no Accessibility permission?" — macOS-only wording in a shared path. |
| O6 | L | `platform.rs:161–174` `open_external` | `explorer.exe <url>` is fine for the strict https allowlist; keep (a `ShellExecuteW` rewrite would need COM init on the helper thread for no gain). |
| O7 | M | `audio.rs:106–123` | Only `F32` and `I16` input streams are handled; WASAPI can report `I32`, `U8`, `I64` etc. for some devices (`cpal-0.16.0/src/host/wasapi/device.rs:197–213`), which currently yields "Microphone unavailable". Make the stream builder generic over `cpal::SizedSample` with `f32: cpal::FromSample<T>`. |

### 1.4 `src-tauri/src/lib.rs` and `src-tauri/src/store.rs`

| # | Sev | Where | Finding |
|---|---|---|---|
| L1 | M | `lib.rs:441–449` `show_settings` Windows branch | No `theme`. tao follows the **system app theme** for the title bar, which is light by default → a white title bar on a dark window. On Windows `theme` is per-window (unlike macOS, where the comment correctly avoids it), so set `config.theme = Some(tauri::Theme::Dark)` in this branch. |
| L2 | M | same | No glass. Tauri applies `windowEffects` on Windows through `window-vibrancy` (`tauri-2.11.5/src/vibrancy/windows.rs`): `Mica`/`MicaDark` need build ≥ 22000 (`window-vibrancy-0.6.0/src/windows.rs:320`), errors are swallowed, and a failed effect leaves a **transparent window with nothing behind it**. So: gate on `windows_version::OsVersion::current().build >= 22000` → `transparent = true`, `window_effects = MicaDark`, `background_color = None`; otherwise today's opaque config. |
| L3 | M | `lib.rs:470–471` | `window.show()` + `set_focus()` immediately after `build()`: on Windows a flash of the pre-load background (black for opaque, see-through for Mica). Show on `PageLoadEvent::Finished` on Windows. |
| L4 | H (UX) | `lib.rs:508–515` tray | `tray.png` is a 44×44 **black** glyph; `icon_as_template(true)` is a no-op outside macOS → nearly invisible on the dark Windows taskbar (and a white glyph would be invisible on a light taskbar). Use the colored `icons/32x32.png` on Windows. `show_menu_on_left_click` defaults to true → left click shows the menu, there is no way to open Settings by (double-)clicking the icon, which is the Windows convention. |
| L5 | H (UX) | `lib.rs:668–672` | Settings opens at startup only when macOS permissions are missing → **never on Windows**. First launch: no window, 670 MB download in the background, tray icon hidden in Windows 11's overflow. Open Settings when `resolve_startup_engine` returns `NeedsDownload` too. |
| L6 | M | `store.rs:111–118` `data_dir` → `app_data_dir()` | On Windows that is the **roaming** `%APPDATA%`; ~1 GB of models would roam with domain profiles, while Tauri already keeps WebView2 data in `%LOCALAPPDATA%\com.dizraid.voice\EBWebView`. `app_local_data_dir()` resolves to the same folder as today on macOS, so switch unconditionally. |
| L7 | M | `store.rs:125–134` `write_atomic` | `fs::rename` over an existing file fails on Windows with `ERROR_SHARING_VIOLATION` (32) / `ERROR_ACCESS_DENIED` (5) while Defender or the indexer holds the file for a moment → settings/history saves silently fail (only logged). The unit test `atomic_writes_use_distinct_temp_files` (8 threads renaming onto one path) will hit this once tests run on Windows. Add a bounded retry. |
| L8 | L | `lib.rs:396–399` log path | `%LOCALAPPDATA%\Voxy.log` sits outside the app folder; move to `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` so one folder holds everything (uninstaller/`install.ps1 -Uninstall` wipe one tree). |
| L9 | L | `lib.rs:580–582` single-instance callback | Fine, but note the tao Alt-tap side effect (see 1.1). |

### 1.5 Build and bundle configuration

| # | Sev | Where | Finding |
|---|---|---|---|
| B1 | M | `src-tauri/tauri.conf.json:57–62` `bundle.icon` | No `icons/icon.ico`. tauri-build falls back to `icons/icon.ico` for the exe resource (`tauri-build-2.6.3/src/lib.rs:608–618`), but the NSIS installer/uninstaller only get a branded icon via `bundle.windows.nsis.installerIcon`/`uninstallerIcon` (bundler `nsis/mod.rs:347,377`); today they show the generic NSIS icon. |
| B2 | L | same file | No `bundle.publisher` → tauri-build and NSIS derive `CompanyName`/Publisher = `dizraid` from the identifier (`lib.rs:648–657`). No `shortDescription`/`longDescription`; no `bundle.windows.nsis.languages` (English only; the author's audience includes Russian speakers). |
| B3 | H (CI) | `src-tauri/build.rs:17–21`, `.github/workflows/ci.yml:107–115` | Unit tests cannot run on Windows: tauri-build embeds the Common Controls v6 manifest via `tauri-winres`/`embed-resource`, which emits bin-only link args, so the **test binary** imports `TaskDialogIndirect` (tao/muda) from comctl32 v5 and dies with `0xC0000139` (tauri-apps/tauri#13419). Fix: `cargo:rustc-link-arg-tests=/MANIFEST:EMBED` + `/MANIFESTINPUT:<manifest>` on MSVC targets. |
| B4 | L | `src-tauri/build.rs`, `ci.yml:39–42`, `release.yml:270–276` | `STATIC_VCRUNTIME` must be set by every caller (CLI 2.11 does, plain `cargo build/test` on Windows does not → CRT mismatch LNK4098 with the /MT sherpa libs). build.rs can default it (`std::env::set_var` before `tauri_build::try_build`; tauri-build reads the env in the same process, `lib.rs:708`). |
| B5 | M | `release.yml:26–28, 349–350, 372–378, 400–405` | A failed Windows build does not block the release and the exe is not checked at all; release notes call Windows experimental and offer no one-command install. |
| B6 | L | `ci.yml:59` | Comment "The Windows port has never been compiled before this job" is stale. |

### 1.6 UI (`ui/`)

| # | Sev | Where | Finding |
|---|---|---|---|
| U1 | M | `ui/settings.js:83–123` | On Windows every key gets the "⌨" cap (`splitKey` L122): the chip reads "⌨ Right Ctrl". Should be "Ctrl / Right Ctrl", "Alt / Right Alt", "Win / Right Win", "⇪ / Caps Lock", "F8". |
| U2 | M | `ui/settings.js` (no handler) | WebView2 keeps browser accelerators: **F5 reloads the Settings page**, Ctrl+P prints, Ctrl+F finds, F3, Ctrl+U view-source. An F-key hotkey pressed while Settings is focused reloads the page. Add a `keydown` guard (harmless on macOS). Also no `contextmenu` suppression (Chromium menu with Back/Reload/Inspect). |
| U3 | M | `ui/settings.css:928–933`, `boot.js` | Only an opaque Windows variant exists; no Mica variant (transparent `html/body`, tinted `.window`, square corners because DWM rounds Win11 windows itself). Needs the backdrop contract (query param → `data-backdrop`). |
| U4 | L | `ui/island.css:12,128`, `ui/settings.css:45–46` | Island font stack has no Segoe entry (falls to Arial on Windows); mono stack lacks `Cascadia Mono`/`Consolas` in island.css. settings.css is fine. |
| U5 | L | `ui/settings.html:103`, `settings.js` | "Launch at login" → Windows wording "Start with Windows"; "Hold Right ⌘" style hints should use the full Windows label ("Hold Right Ctrl and speak", already the case via `renderEmptyHint` L1183–1191, but the cap logic changes in U1 must keep it). |
| U6 | M | `ui/settings.js:264–285` | No warning when the hotkey is a Win key (A5). |
| U7 | L | `ui/boot.js:7`, `ui/island.js:7–8`, `ui/settings.js:9,1520` | Platform detection is duplicated (`navigator.platform` in three places); the browser demo cannot preview the Windows look. Single source: `boot.js` sets `data-platform` (overridable with `?platform=win|mac` outside Tauri) and `settings.js` reads it. |

### 1.7 Docs

`BUILD_WINDOWS.md:3–8` (status), `:75` (`Voxy_0.1.0_x64-setup.exe`), `:119–139` (first launch, data paths, log), `:164–179` (known gaps); `README.md:8,31,199–203,209`; `docs/DEVELOPMENT.md:97,177–187`; `docs/TROUBLESHOOTING.md:18,107–116`; `install.sh:237` (non-macOS message); `.github/workflows/release.yml:167–169` (notes). All updated in Phase 2 / WS-E.

### 1.8 Things verified as fine (no change)

- `keyring = "4.2"` default feature `v1` enables `windows-native-keyring-store` (credential target `api-key:<provider>.com.dizraid.voice` in Credential Manager) and the Apple keychain store.
- `enigo::Key::Other(0x56)` maps to `VIRTUAL_KEY(0x56)` (`keycodes.rs:1074–1078`); `Key::Control` → `VK_CONTROL`; enigo returns `Err` when `SendInput` inserts fewer events than requested (UIPI).
- `arboard::SetExtWindows::exclude_from_history` exists and sets `CanIncludeInClipboardHistory = 0` (Win+V history excluded).
- `tauri-plugin-autostart` on Windows writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Voxy` (`PackageInfo.name` = `productName`).
- `tauri-plugin-single-instance` on Windows: named mutex + `WM_COPYDATA`; the callback runs in the first instance.
- `Url::join("settings.html?backdrop=mica")` keeps the query (`tauri-2.11.5/src/manager/webview.rs:452–456`), so a query parameter on `WebviewUrl::App` works.
- `ureq 2` uses rustls with bundled roots; no Windows cert-store dependency. Model paths stay far below `MAX_PATH`; Rust's `std::fs` uses `\\?\` for long paths anyway.
- NSIS (`currentUser` mode): installs to `%LOCALAPPDATA%\Voxy`, writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Voxy` (`UninstallString`, `DisplayIcon`, `DisplayVersion`), kills a running `Voxy.exe` silently under `/S` and `/P`, offers "Run Voxy" on the finish page, has a "delete app data" checkbox in the uninstaller. Verified in tauri-bundler at tag `tauri-cli-v2.11.5`.

---

## 2. Scope

**In**

- All findings in §1 (hook robustness, clipboard parity, sounds, UIPI diagnostics, audio formats, island/settings window fixes, tray, first run, data dir, atomic writes, bundle config, test manifest, CI tests + smoke run, release gating, `install.ps1`, UI parity, docs, `CLAUDE.md`).

**Out (one-line reasons)**

- **Windows code signing / SmartScreen reputation**: needs a certificate or Azure Trusted Signing account; documented as roadmap.
- **Auto-updater** (`tauri-plugin-updater`): needs signing keys and a feed; `install.ps1` re-run is the update path, like `install.sh`.
- **ARM64 Windows**: separate target, no hardware; the x64 installer runs under emulation (installer warns).
- **GPU inference (CUDA/DirectML)**: already deferred in `BUILD_WINDOWS.md`; needs shared-library sherpa builds.
- **Custom (HTML) title bar on Windows**: visually closer to macOS but unverifiable (resize/snap/hit-testing/DPI); native dark title bar + Mica is the robust choice.
- **winget manifest**: requires a public repo and stable release cadence; later.
- **Windows 7/8.1**: Tauri 2 / WebView2 require Windows 10.
- **Light theme**: separate roadmap item.
- **Clipboard-free paste (typing text with `SendInput` unicode)**: changes behavior on both platforms; not a port concern.

---

## 3. Phase 0 — foundation (orchestrator, sequential, before fan-out)

Also added in Phase 0: `scripts/cross-check.sh`, the Linux cross type-check used for every verification step below.

After Phase 0 the files `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json` are **frozen**: no workstream may touch them. If a workstream discovers it needs something else there, it stops and reports.

### 3.1 `src-tauri/Cargo.toml`

Replace the Windows target block (lines 45–50) with:

```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Media_Audio",
    "Win32_Security",             # GetTokenInformation: is the foreground app elevated (UIPI)?
    "Win32_System_DataExchange",  # clipboard sequence number and formats
    "Win32_System_LibraryLoader", # GetModuleHandleW for the keyboard hook
    "Win32_System_Threading",     # hook thread priority, OpenProcess/OpenProcessToken
] }
# Windows build number: Mica needs Windows 11 (build 22000+). Already in Cargo.lock as
# a dependency of tauri; reads RtlGetVersion, so no manifest tricks are needed.
windows-version = "0.1"
```

(Verified in `windows-sys-0.59.0`: `GetClipboardSequenceNumber`, `RegisterClipboardFormatW`, `IsClipboardFormatAvailable` → `Win32_System_DataExchange`; `GetModuleHandleW` → `Win32_System_LibraryLoader`; `SetThreadPriority`, `GetCurrentThread`, `OpenProcess`, `OpenProcessToken`, `THREAD_PRIORITY_HIGHEST`, `PROCESS_QUERY_LIMITED_INFORMATION` → `Win32_System_Threading`; `GetTokenInformation`, `TokenElevation`, `TOKEN_ELEVATION`, `TOKEN_QUERY` → `Win32_Security`; `SetTimer`, `KillTimer`, `UnhookWindowsHookEx`, `GetForegroundWindow`, `GetWindowThreadProcessId`, `LLKHF_INJECTED` → `Win32_UI_WindowsAndMessaging`; `PlaySoundW`, `SND_*` → `Win32_Media_Audio`; `CloseHandle`, `GetLastError` → `Win32_Foundation`; `w!` macro → crate root.)

### 3.2 `src-tauri/Cargo.lock`

Add only the new edge (`windows-version 0.1.7` is already locked as a tauri dependency):

```sh
cd /home/user/voxy/src-tauri && cargo update --workspace --offline
git diff --stat Cargo.lock   # expect: only the [[package]] name = "voxy" block gains "windows-version"
```

If the diff touches any other package, `git checkout Cargo.lock` and run `cargo update --workspace` with network instead; the result must still be that single edge.

### 3.3 `src-tauri/tauri.conf.json` — `bundle` section only

Leave `app.windows[0]` untouched (the `settings_window_is_glass` test asserts it; the Windows variant is derived at runtime in `show_settings`). Replace the `bundle` object with:

```json
"bundle": {
  "active": true,
  "targets": ["app", "nsis"],
  "category": "Productivity",
  "publisher": "DIZRAID",
  "homepage": "https://github.com/DIZRAID/voxy",
  "shortDescription": "Push-to-talk dictation",
  "longDescription": "Voxy — push-to-talk dictation with on-device speech recognition.",
  "icon": [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/128x128@2x.png",
    "icons/icon.icns",
    "icons/icon.ico"
  ],
  "macOS": {
    "minimumSystemVersion": "14.0",
    "entitlements": "Entitlements.plist"
  },
  "windows": {
    "webviewInstallMode": { "type": "downloadBootstrapper", "silent": true },
    "nsis": {
      "installMode": "currentUser",
      "installerIcon": "icons/icon.ico",
      "uninstallerIcon": "icons/icon.ico",
      "languages": ["English", "Russian"],
      "displayLanguageSelector": false
    }
  }
}
```

`icons/icon.ico` exists (6 sizes, 16–256 px). Validate with `python3 -m json.tool src-tauri/tauri.conf.json >/dev/null`.

### 3.4 Contracts between workstreams (write these into each brief)

- **C1 backdrop** (WS-C → WS-D): on Windows `show_settings` loads `settings.html?backdrop=mica` (Win11, Mica applied) or `settings.html?backdrop=opaque` (Win10 fallback). `ui/boot.js` copies the value to `document.documentElement.dataset.backdrop` (`data-backdrop="mica"|"opaque"`); absent on macOS.
- **C2 log tokens** (WS-A, WS-C → WS-E smoke test): hotkey.rs keeps the substring `хук установлен` in its success line; lib.rs logs a line containing `startup complete` after the tray and island exist.
- **C3 rename retry** (WS-C → WS-E): `store::write_atomic` retries `rename` on Windows error codes 5/32, so `atomic_writes_use_distinct_temp_files` can run on Windows.
- **C4 paste error** (WS-B internal): `output::insert_text` returns an `anyhow::Error` that downcasts to a new `output::PasteBlocked` when the foreground app is elevated; `worker.rs` (also WS-B) maps it to the island message `Paste blocked by an admin app — text kept in clipboard`.
- **C5 island placement on Windows** (WS-C → WS-D; added after review by the repo owner): Windows has no camera notch, so the island is a floating pill **bottom-center above the taskbar** (like the Windows 11 volume flyout), not at the top. Rust puts the 520×150 window's bottom edge on the primary monitor's work-area bottom (physical coordinates from `work_area()`, so a taskbar docked elsewhere or auto-hidden still works); the page anchors the pill to the window's bottom with a ~12 px gap under `html[data-platform="win"]`, rises in from below on enter (fade + slight scale), sinks out on exit, and uses opacity-only fades under `prefers-reduced-motion`. `IslandMetrics`/`island_metrics` stay unchanged. This supersedes the top-of-work-area placement in WS-C task 9.
- No new Tauri commands or capabilities are needed; `app_commands.rs` and `capabilities/*.json` stay untouched.

### 3.5 Phase 0 verification

```sh
scripts/cross-check.sh win check --all-targets
scripts/cross-check.sh mac check --all-targets
```

Both must pass (an unused-dependency warning for `windows-version` is not emitted by rustc; fine). Then fan out.

---

## 4. Phase 1 — parallel workstreams

### 4.0 File ownership (disjoint)

| Workstream | Owns (may edit) | Creates |
|---|---|---|
| **WS-A Hotkey** | `src-tauri/src/hotkey.rs` | — |
| **WS-B Platform / output / audio** | `src-tauri/src/platform.rs`, `src-tauri/src/output.rs`, `src-tauri/src/worker.rs`, `src-tauri/src/audio.rs` | — |
| **WS-C Windows shell** | `src-tauri/src/lib.rs`, `src-tauri/src/island.rs`, `src-tauri/src/store.rs` | — |
| **WS-D UI** | `ui/boot.js`, `ui/settings.js`, `ui/settings.css`, `ui/settings.html`, `ui/island.js`, `ui/island.css`, `ui/island.html` | — |
| **WS-E Build / CI / release / installer** | `src-tauri/build.rs`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `install.sh` (one message only) | `src-tauri/windows/tests.manifest`, `install.ps1` |
| Frozen (Phase 0) | `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json` | |
| Phase 2 (orchestrator) | `README.md`, `BUILD_WINDOWS.md`, `docs/DEVELOPMENT.md`, `docs/TROUBLESHOOTING.md`, `SECURITY.md`, `.github/ISSUE_TEMPLATE/bug_report.md` | `CLAUDE.md`, `docs/WINDOWS_PORT_PLAN.md` |

Not touched by anyone: `src-tauri/src/{main,asr,models,online,discovery,app_commands}.rs`, `src-tauri/capabilities/*`, `src-tauri/examples/*`, `src-tauri/icons/*`, `.github/scripts/*`, `docs/marketing/*`.

### 4.1 Common preamble (paste at the top of every brief)

> You are implementing one workstream of the Windows port of **Voxy**, a push-to-talk dictation app: Tauri 2 + Rust in `src-tauri/`, plain static HTML/CSS/JS in `ui/` (no npm, no bundler). Repository: `/home/user/voxy`. Read `docs/WINDOWS_PORT_PLAN.md` §0–§2 for context, then your brief below. Code style: dense explanatory comments (Russian comments in Rust are the norm and fine; English is fine too), no emojis, English in docs/UI strings.
>
> **Hard constraints**
> - You can only edit the files listed under "Owned files". Do not touch `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json` or any other workstream's files. If you need something there, stop and report what and why.
> - There is **no Windows or macOS machine**. Nothing can be run or linked. Verification is cross type-checking from Linux with `scripts/cross-check.sh` (zig as C compiler, stubbed windres, nothing linked; see the script header):
>   - Windows: `scripts/cross-check.sh win check --all-targets` and `scripts/cross-check.sh win clippy --all-targets` (target `x86_64-pc-windows-gnu`; real releases use MSVC — flag anything where gnu vs msvc could differ).
>   - macOS: `scripts/cross-check.sh mac check --all-targets` and `scripts/cross-check.sh mac clippy --all-targets` (target `aarch64-apple-darwin`; macOS must not regress).
>   - `cargo test` cannot run anywhere here (no webkit2gtk on Linux); unit tests you add run later in CI on macOS and Windows. Write them anyway: put decision logic in `#[cfg]`-free pure functions and test those; keep `#[cfg(target_os = …)]` to the thin OS-call layer.
>   - Baseline: both scripts pass; clippy has exactly 2 pre-existing `manual_checked_ops` warnings. You must not add warnings.
> - Reason from documented API semantics (Win32 docs, crate sources in `~/.cargo/registry/src/*/`). Where behavior cannot be verified, prefer conservative code plus a log line. Never log or take locks inside a Win32 hook callback or audio callback.
> - Do not commit; leave changes in the working tree. Do not create build outputs outside `src-tauri/target/` (gitignored). Temporary files go to a folder outside the repository (e.g. `mktemp -d`).
> - JS syntax can be checked with `node --check ui/<file>.js`; JSON with `python3 -m json.tool`.
>
> **Report format (final message):** (1) what changed per file, (2) anything you could not verify and why, (3) gnu-vs-msvc or Tauri-version caveats, (4) "Docs notes": every user-visible behavior change, for the documentation pass.

### 4.2 WS-A — Global hotkey hook (`src-tauri/src/hotkey.rs`)

**Goal.** Make the `WH_KEYBOARD_LL` listener (lines 685–764) robust: correct API contract, immune to Voxy's own synthetic input, resistant to silent hook removal, diagnosable from the log. The recording state machine, `Capture`, `KEY_TABLE`, `on_press`/`on_release` and the macOS code stay as they are.

**Owned files.** `src-tauri/src/hotkey.rs` only.

**Verified facts you can rely on**
- enigo (used by `output::send_paste`) stamps every injected event with `dwExtraInfo = enigo::EVENT_MARKER` (`pub const EVENT_MARKER: u32 = 100` in `enigo-0.6.1/src/lib.rs:87`; `win_impl.rs:395` uses it when `Settings::windows_dw_extra_info` is `None`, which `output.rs` leaves default). `enigo` is already a dependency, so `enigo::EVENT_MARKER` is importable.
- tao injects a synthetic Left Alt tap (`VK_LMENU`, `dwExtraInfo` 0, `LLKHF_INJECTED`) in `force_window_active` when `SetForegroundWindow` is refused. Do **not** filter all `LLKHF_INJECTED` events (PowerToys/AutoHotkey remaps are injected too); only Voxy's marker.
- Low-level hooks are silently removed by Windows if the callback exceeds `LowLevelHooksTimeout`; the callback runs on the installing thread, which must pump messages. `SetTimer(NULL, 0, ms, None)` posts `WM_TIMER` to that thread's queue (`msg.hwnd` null), retrievable with `GetMessageW(&mut msg, NULL, 0, 0)` without any window.
- windows-sys features available after Phase 0: `Win32_System_LibraryLoader` (`GetModuleHandleW`), `Win32_System_Threading` (`GetCurrentThread`, `SetThreadPriority`, `THREAD_PRIORITY_HIGHEST`), `Win32_UI_WindowsAndMessaging` (`SetTimer`, `KillTimer`, `UnhookWindowsHookEx`, `WM_TIMER`, `HHOOK`, `LLKHF_INJECTED`, `KBDLLHOOKSTRUCT`), `Win32_Foundation` (`GetLastError`).

**Tasks**
1. `SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0)`. Log `GetLastError()` on failure.
2. Spawn the hook thread with `std::thread::Builder::new().name("voxy-hotkey-hook".into())`; first thing on it: `SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST)` (log if it returns 0, continue).
3. In `hook_proc`, before any state change: `if kb.dwExtraInfo == OWN_INPUT_MARKER { return CallNextHookEx(...) }` with `const OWN_INPUT_MARKER: usize = enigo::EVENT_MARKER as usize;`. Keep the callback free of logging, allocation beyond the channel send, and locks (document this in a comment; cite the timeout).
4. Self-healing: after installing the hook, `SetTimer(null_mut(), 0, REHOOK_INTERVAL_MS /* 60_000 */, None)`. In the message loop, on `msg.message == WM_TIMER && msg.hwnd.is_null()`: if every word of `ctx.down` is 0 (no key held, so no release is lost), `UnhookWindowsHookEx(hook)` then `SetWindowsHookExW(...)` again; on failure `log::error!` with `GetLastError()` and retry at the next tick; log successful re-hooks at `debug` only. Keep the "exactly one hook at a time" invariant (always unhook before hooking). `TranslateMessage`/`DispatchMessageW` are not needed (no windows on this thread), but calling them is harmless.
5. If `GetMessageW` returns 0 or -1, `log::error!` ("message loop ended, the hotkey is off until restart") and exit the thread. Keep the success message containing `хук установлен` (contract C2).
6. Store the current `HHOOK` in a local on the hook thread (no global needed; `CallNextHookEx` ignores its first argument).
7. Module docs: add a Windows section mirroring the macOS invariants (one hook per process, no locks/logging in the callback, timeout, injected-input filter, periodic re-hook) and the user-facing caveats: a Win key tap opens the Start menu and Left Alt is intercepted by apps' menus and by tao's focus hack, so the UI warns about Win keys and docs recommend Right Ctrl / F-keys.
8. Tests: extract the decision "is this event Voxy's own injected input?" and "may we re-hook now (no key held)?" into tiny pure functions compiled on all platforms (`fn is_own_input(extra_info: usize) -> bool`, `fn no_key_held(words: &[AtomicU64; 4]) -> bool`) and unit-test them next to the existing tests (they run in CI on macOS and Windows).

**Do not**
- Change `KEY_TABLE`, `ESCAPE_CODE`, `Capture`, `spawn`, the controller thread, or any macOS code.
- Swallow keys (the hook stays listen-only; always `CallNextHookEx`).
- Filter by `LLKHF_INJECTED` alone.

**Acceptance**
- `scripts/cross-check.sh win check --all-targets`, `scripts/cross-check.sh win clippy --all-targets`, `scripts/cross-check.sh mac check --all-targets`, `scripts/cross-check.sh mac clippy --all-targets` all pass with no new warnings.
- `hook_proc` contains no `log::`, no `Mutex`, no `String` formatting.
- Report includes a short paragraph on `LowLevelHooksTimeout` reasoning and the re-hook interval choice.

### 4.3 WS-B — Platform seam, paste, audio (`platform.rs`, `output.rs`, `worker.rs`, `audio.rs`)

**Goal.** Give Windows the same clipboard safety as macOS, fix the sound-alias lifetime, make UIPI failures diagnosable, and accept every WASAPI sample format.

**Owned files.** `src-tauri/src/platform.rs`, `src-tauri/src/output.rs`, `src-tauri/src/worker.rs` (only the paste-error match at lines 323–334), `src-tauri/src/audio.rs`.

**Verified facts**
- `GetClipboardSequenceNumber()` (Win32_System_DataExchange) increments on every clipboard write by anyone; returns 0 when the caller has no `WINSTA_ACCESSCLIPBOARD`.
- `RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))` + `IsClipboardFormatAvailable(fmt)` work without `OpenClipboard`. arboard's `exclude_from_history()` sets only `CanIncludeInClipboardHistory` (`arboard-3.6.1/src/platform/windows.rs:834–839`), so Voxy's own writes never look like secrets.
- `windows_sys::w!("…")` yields a static null-terminated UTF-16 pointer; `PlaySoundW` flags `SND_ALIAS | SND_ASYNC | SND_NODEFAULT` are in `Win32_Media_Audio`.
- UIPI: `SendInput` returns 0 when blocked and the error is not reported; enigo then returns `InputError::Simulate`. Elevation check: `GetForegroundWindow` → `GetWindowThreadProcessId(hwnd, &mut pid)` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)` → `OpenProcessToken(h, TOKEN_QUERY, &mut tok)` → `GetTokenInformation(tok, TokenElevation, &mut TOKEN_ELEVATION, size, &mut ret)` → `CloseHandle` both. Any failure → `false`.
- cpal 0.16: `device.build_input_stream::<T, _, _>` for any `T: cpal::SizedSample`; convert with `f32::from_sample(s)` where `f32: cpal::FromSample<T>`. WASAPI shared mode usually gives `F32` or `I16`, sometimes `I32`/`U8`/`I64` (`cpal-0.16.0/src/host/wasapi/device.rs:197–213`).

**Tasks**
1. `platform.rs` — Windows `clipboard_generation()`: `match unsafe { GetClipboardSequenceNumber() } { 0 => None, n => Some(n as isize) }`. Update the doc comment (it says "Windows: not implemented yet").
2. `platform.rs` — Windows `clipboard_holds_secret()`: register the format once (`OnceLock<u32>`), `fmt != 0 && IsClipboardFormatAvailable(fmt) != 0`. Comment: set by 1Password/Bitwarden/KeePassXC; Voxy's own writes do not set it.
3. `platform.rs` — `play`: static aliases via `w!`, flags `SND_ALIAS | SND_ASYNC | SND_NODEFAULT`; comment on the string lifetime.
4. `platform.rs` — new `#[cfg(target_os = "windows")] pub fn foreground_is_elevated() -> bool` (and a `false` stub for other platforms, used only on Windows but keep it cfg-free-callable). Log at `debug` which step failed, never panic.
5. `output.rs` — add `#[derive(Debug)] pub struct PasteBlocked;` implementing `Display` + `std::error::Error` ("the active window runs elevated; Windows (UIPI) blocks synthetic input"). In `insert_text`, when `send_paste()` fails **and** `crate::platform::foreground_is_elevated()`, return `Err(anyhow::Error::new(PasteBlocked))` (text stays in the clipboard, as today). Fix the macOS-specific wording in `send_paste`'s `context(...)`: make it platform-neutral ("enigo: could not send the paste shortcut (macOS: Accessibility permission?)").
6. `worker.rs` lines 323–334 — map the error: `if e.downcast_ref::<output::PasteBlocked>().is_some() { "Paste blocked by an admin app — text kept in clipboard" } else { "Paste failed — text kept in clipboard" }`. Keep the `log::error!("вставка: {e:#}")`.
7. `audio.rs` — replace the two-arm `match supported.sample_format()` (lines 106–123) with a generic helper `fn build_stream<T>(device, config, channels, on_chunk, err_cb) -> Result<cpal::Stream> where T: cpal::SizedSample, f32: cpal::FromSample<T>` and arms for `F32, I16, I32, I64, I8, U8, U16, U32, U64, F64` (`_ => Err(...)` for anything new). Keep the realtime-hygiene comment: the callback only converts, downmixes and pushes.
8. Tests: `PasteBlocked` downcast round-trip (pure, all platforms); a `downmix`/conversion test if cheap. Do not add tests that need a clipboard or an audio device.

**Do not**
- Change `insert_text`'s timing constants or clipboard restore logic beyond using the now-real generation check.
- Touch `platform::open_external` (keep `explorer`), permission functions, `inference_threads`, or macOS code paths.
- Touch `worker.rs` outside the paste-error arm.

**Acceptance**
- Four cross-check commands pass, no new warnings (watch `unsafe` blocks: each with a `// SAFETY:` comment, as the file already does).
- `grep -n "not implemented\|пока не реализовано" src-tauri/src/platform.rs` returns nothing for the clipboard functions.
- Report lists the exact island strings added (for docs) and the WASAPI formats now accepted.

### 4.4 WS-C — Windows shell: settings window, island, tray, startup, data paths (`lib.rs`, `island.rs`, `store.rs`)

**Goal.** Make the Settings window look native (dark title bar, Mica on Windows 11, opaque on Windows 10), show it on first run, give Windows a visible tray icon with Windows click conventions, remove island flashes/focus stealing, and keep all data in `%LOCALAPPDATA%\com.dizraid.voice`.

**Owned files.** `src-tauri/src/lib.rs`, `src-tauri/src/island.rs`, `src-tauri/src/store.rs`.

**Verified facts**
- Tauri applies `windowEffects` on Windows via window-vibrancy right after window creation (`tauri-2.11.5/src/window/mod.rs:444`, `src/vibrancy/windows.rs`); `Mica`/`MicaDark` require build ≥ 22000 (`window-vibrancy-0.6.0/src/windows.rs:320`), uses `DWMWA_SYSTEMBACKDROP_TYPE` on ≥ 22523 and the older `DWMWA_MICA_EFFECT` below. Errors are swallowed, so gate in Rust with `windows_version::OsVersion::current().build >= 22000` (crate added in Phase 0).
- `WindowConfig` fields you set at runtime: `transparent: bool`, `window_effects: Option<WindowEffectsConfig { effects: Vec<WindowEffect>, state, radius, color }>`, `background_color: Option<tauri::window::Color>`, `theme: Option<tauri::Theme>`, `url: WebviewUrl`. `WebviewUrl::App("settings.html?backdrop=mica".into())` keeps the query (`manager/webview.rs:452–456`). On Windows `theme` is per-window (tao: `DWMWA_USE_IMMERSIVE_DARK_MODE` + WebView2 color scheme), unlike macOS where `theme` is global (that is why the config leaves it unset — keep it that way for macOS).
- `WebviewWindowBuilder::{focused(false), visible(false), on_page_load(|window: WebviewWindow, payload| …)}` exist; `tauri::webview::PageLoadEvent::Finished`.
- `Monitor::work_area() -> &PhysicalRect<i32, u32>` (fields `position`, `size`) exists in tauri 2.11.5.
- Tray: `TrayIconBuilder::show_menu_on_left_click(bool)`, `.on_tray_icon_event(|tray, event| …)` with `TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }` and `TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }` (`tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent}`); `tray.app_handle()` gives the `AppHandle`. `icon_as_template` is macOS-only.
- `app.path().app_local_data_dir()` = `%LOCALAPPDATA%\com.dizraid.voice` on Windows and `~/Library/Application Support/com.dizraid.voice` on macOS (same as `app_data_dir()` there).
- Tests in `lib.rs` (`settings_window_is_glass`, `ipc_is_scoped_per_window`, `csp_is_strict`, …) parse `tauri.conf.json`, `capabilities/` and the `generate_handler![…]` list; keep them passing (do not reorder/rename the handler list).

**Tasks**
1. **`show_settings` (lib.rs 421–475).** Extract a `#[cfg]`-free pure function `fn windows_settings_config(mut config: WindowConfig, mica: bool) -> WindowConfig` that sets `theme = Some(Theme::Dark)` and either (mica) `transparent = true`, `window_effects = Some(WindowEffectsConfig { effects: vec![WindowEffect::MicaDark], state: None, radius: None, color: None })`, `background_color = None`, `url = App("settings.html?backdrop=mica")`, or (opaque) today's values plus `url = App("settings.html?backdrop=opaque")`. Call it only under `cfg!(target_os = "windows")` with `mica = windows_version::OsVersion::current().build >= 22000` (wrap the version call in `#[cfg(target_os = "windows")]`). Log which variant was chosen with the build number. Unit-test the pure function (both branches) in `mod tests`.
2. **Windows show-on-load.** On Windows, build with `.on_page_load(move |window, payload| if payload.event() == PageLoadEvent::Finished { let _ = window.show(); let _ = window.set_focus(); })` and skip the immediate `show()/set_focus()`; macOS keeps the current immediate show. The `Focused`/`Destroyed` handlers for `Capture` stay for both.
3. **Tray (lib.rs 499–524).** Windows: icon from `include_bytes!("../icons/32x32.png")` (colored app icon; readable on light and dark taskbars; the macOS template glyph stays for macOS), `icon_as_template` only under `cfg!(target_os = "macos")`, `.show_menu_on_left_click(false)` on Windows, and `.on_tray_icon_event` opening Settings on left `Click` with `button_state == Up` and on left `DoubleClick` (no-op elsewhere; harmless on macOS since left click shows the menu there — gate the handler body with `cfg!(target_os = "windows")`).
4. **First run (lib.rs 659–672).** Compute `let needs_download = matches!(active, StartupEngine::NeedsDownload(_));` before the `match`, and open Settings when `!accessibility || !input_monitoring || needs_download`. Comment: on Windows there are no permissions, so this is the only thing that makes the first launch visible.
5. **Startup marker (contract C2).** After `build_tray(app)?` and `island::create`, `log::info!("startup complete: tray and island ready")`.
6. **Log path (lib.rs 386–416).** Windows: `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` (`create_dir_all` the folder; the identifier string is duplicated here on purpose — `init_logging` runs before Tauri exists; add a comment pointing at `tauri.conf.json`). macOS unchanged.
7. **`store::data_dir` (store.rs 111–118).** Use `app.path().app_local_data_dir()` unconditionally with a comment (roaming `%APPDATA%` would sync ~1 GB of models; WebView2 data already lives in the local folder; identical path on macOS). Update the temp-dir fallback comment.
8. **`store::write_atomic` (store.rs 125–134).** Retry `fs::rename` up to 5 times with growing sleeps (20, 40, … ms) when `e.raw_os_error()` is 5 or 32 on Windows (`#[cfg(windows)] const RETRYABLE: &[i32] = &[5, 32]; #[cfg(not(windows))] const RETRYABLE: &[i32] = &[];`). Keep the temp-file cleanup. Comment why (Defender/indexer; the concurrent unit test).
9. **Island (island.rs 151–176).** `.focused(false)`, `.visible(false)`, `.on_page_load(... Finished => show())`; position with physical coordinates from `monitor.work_area()`: `x = wa.position.x + ((wa.size.width as f64 - WIDTH * scale) / 2.0) as i32`, `y = wa.position.y` (`tauri::Position::Physical`), applied before show; log the chosen rect and scale. Keep `set_ignore_cursor_events(true)`, `always_on_top`, `skip_taskbar`, `focusable(false)`, `shadow(false)`, `decorations(false)`, `transparent(true)`. Add a comment that `visible_on_all_workspaces` is a no-op on Windows (keep or remove, your call). Update the file's module doc (it only describes the NSPanel).
10. **Tests (lib.rs).** Add `windows_bundle_is_configured`: parse `tauri.conf.json` and assert `bundle.icon` contains `icons/icon.ico`, `bundle.publisher == Some("DIZRAID")`, `bundle.windows.nsis` has `installer_icon` and `languages` containing `"English"` and `"Russian"`, and `webview_install_mode` is `DownloadBootstrapper`. Add the `windows_settings_config` tests from task 1.

**Do not**
- Touch the macOS branches' behavior (`force_dark_appearance`, NSPanel, activation policy), the handler list, `app_commands.rs`, capabilities, or `tauri.conf.json`.
- Add a custom title bar or `decorations(false)` for the settings window.
- Use `window-vibrancy` directly (Phase 0 did not add it; Tauri's config path is enough).

**Acceptance**
- Four cross-check commands pass, no new warnings.
- `grep -n "startup complete" src-tauri/src/lib.rs` finds the marker; `grep -n "backdrop=" src-tauri/src/lib.rs` shows both variants.
- Report lists: new data/log paths, first-run behavior, tray behavior, Win10 vs Win11 appearance (for docs), and anything about DPI/multi-monitor you could not verify.

### 4.5 WS-D — Settings and island UI for Windows (`ui/*`)

**Goal.** Make the pages behave and read correctly on Windows: platform-aware key caps and hints, Mica/opaque backdrop variants, native fonts, no browser accelerators or context menus, Windows wording, and a browser-preview switch for the Windows look.

**Owned files.** `ui/boot.js`, `ui/settings.js`, `ui/settings.css`, `ui/settings.html`, `ui/island.js`, `ui/island.css`, `ui/island.html`.

**Verified facts / contracts**
- Contract C1: on Windows the page URL carries `?backdrop=mica` or `?backdrop=opaque` (WS-C); macOS has no parameter. `navigator.platform` is `"Win32"` in WebView2.
- CSP (`tauri.conf.json`, unit test `csp_is_strict`) forbids inline `<script>` and `style=""` attributes: all scripts stay in files, styles via CSSOM only. The unit test also greps `ui/*.html` for `<script>` and ` style=`.
- Windows 11 rounds top-level windows itself; Mica shows through wherever the page is transparent. Windows 10 fallback is an opaque `#1a162b` window (`backgroundColor` set in Rust) with the native title bar.
- WebView2 keeps browser accelerators (F5, Ctrl+R, Ctrl+P, Ctrl+F, F3, Ctrl+U, Ctrl+Shift+I) and shows Chromium's context menu unless the page prevents them. Hotkey capture happens in Rust from a global hook, so preventing defaults in the page never affects capture.
- Windows wording: "Start with Windows" (autostart), keys "Ctrl", "Alt", "Win", "Shift", "Caps Lock".

**Tasks**
1. **`boot.js`** — single source of truth: set `root.dataset.platform` from `navigator.platform`, but outside Tauri allow `?platform=win|mac` to override (demo). Copy `?backdrop=` (only values `mica`/`opaque`) to `root.dataset.backdrop`; outside Tauri default `backdrop` to `opaque` when platform is `win` unless the query says otherwise.
2. **`settings.js`** — `IS_MAC` reads `document.documentElement.dataset.platform === "mac"` (boot.js runs first; keep the `navigator` fallback only if the attribute is missing). Same for the demo's `mac` at L1520. Replace `splitKey` for Windows: caps map `{Control*: "Ctrl", Alt/AltGr: "Alt", Meta*: "Win", Shift*: "Shift", CapsLock: "⇪"}`, F-keys → cap "F8"-style with label "F8" (or cap "Fn"-neutral "⌨" and label — pick one and keep `renderEmptyHint` saying the full label on Windows: "Hold Right Ctrl and speak"). Keep macOS output identical to today (existing strings and tests of the demo).
3. **Win-key warning** — in `renderHotkey`, when `!IS_MAC && /^Meta(Left|Right)$/.test(hotkey)`, set the hotkey row's `.row-sub` to "Tapping Win alone opens the Start menu — Right Ctrl or an F-key works better" and add a `warn` class (amber `--warn`); restore "Hold to dictate" otherwise. Optional second hint for `Alt` (Left Alt): "Apps use Left Alt for menus".
4. **Accelerator guard + context menu** (both pages): `keydown` capture handler that `preventDefault()`s F5, F3, F12, Ctrl/Cmd+R, Ctrl/Cmd+P, Ctrl/Cmd+F, Ctrl/Cmd+U, Ctrl+Shift+I/J, Ctrl+G; `contextmenu` preventDefault except inside `input, textarea`. Put it in `boot.js` for settings (island has no boot.js: add it at the top of `island.js`, or a tiny shared `ui/guard.js` loaded by both pages via `<script src>` — your call; both HTML files are yours).
5. **`settings.css`** — split the Windows block (L928–933) into: common Windows rules (`.drag-strip { display:none }`, `.window { border-radius: 0; box-shadow: none }`, `.sidebar { padding-top: 12px }`, title-bar offsets if needed), opaque variant (`html[data-platform="win"]:not([data-backdrop="mica"]) .window { background: var(--opaque-bg) }`, html/body opaque as now), and Mica variant (`html[data-platform="win"][data-backdrop="mica"]:not([data-demo])` and its `body` → `background: transparent`; `.window` keeps the tint `rgba(14,10,26,var(--glass-tint))`; consider `--glass-tint: 0.5` for Mica since Mica is lighter than macOS vibrancy — document it as an unverified starting value, like the macOS comment does). Keep `color-scheme: dark`. Check the `.title-bar/.page-title` and `.content` mask offsets look right without the 46 px drag strip (reduce the top gap on Windows).
6. **`island.css`** — font stacks: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", sans-serif` and mono `ui-monospace, "SF Mono", Menlo, "Cascadia Mono", Consolas, monospace`. Nothing else: the idle/recording geometry already handles `data-platform="win"`/`data-notch="no"`.
7. **`island.js`** — read platform from `?platform=` override outside Tauri like boot.js (keep `navigator` detection inside Tauri).
8. **`settings.html`** — default cap text "⌘" is fine (JS replaces it); set the autostart row title from JS on Windows ("Start with Windows", sub "Start Voxy when you sign in"), or add `data-win-title` attributes read by JS — avoid inline styles. The permissions banner stays hidden on Windows (backend reports both true); no markup change needed.
9. **Demo** — `?platform=win&backdrop=mica|opaque` must render the Windows layouts in the browser preview (`python3 -m http.server 4173 --directory ui`), including a simple painted "title bar" placeholder only under `[data-demo]` if you want; the demo must still default to the macOS look.

**Do not**
- Add inline scripts or `style=` attributes; add remote fonts; change IPC calls or event names; touch `docs/marketing/*`.
- Change macOS visuals or strings (diff the macOS branch of every change).

**Acceptance**
- `node --check ui/boot.js ui/settings.js ui/island.js` pass; `grep -n "<script>\| style=" ui/*.html` prints nothing.
- Both cross-check scripts still pass (they compile `include_str!`'d HTML into tests; no Rust change expected).
- Report includes the exact new user-facing strings and the CSS variables/selectors for the Windows variants (for docs and future screenshots).

### 4.6 WS-E — Build, CI, release, installer (`build.rs`, workflows, `install.ps1`)

**Goal.** Run the unit tests on Windows in CI, add a first runtime signal (smoke run), make the Windows installer a required release artifact with a one-command PowerShell installer, and default the static CRT on MSVC.

**Owned files.** `src-tauri/build.rs`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `install.sh` (only the non-macOS error message at line 237). New: `src-tauri/windows/tests.manifest`, `install.ps1`.

**Verified facts**
- tauri-build embeds the app manifest + icon through `tauri-winres` → `embed_resource::compile`, which links the resource into **binaries only**; test binaries lack the Common Controls v6 manifest, hence `0xC0000139` (tauri-apps/tauri#13419). Cargo supports `cargo:rustc-link-arg-tests=FLAG` (test targets only). MSVC `link.exe` accepts `/MANIFEST:EMBED` + `/MANIFESTINPUT:<file>`; rustc adds no manifest of its own. Default manifest content: `tauri-build-2.6.3/src/windows-app-manifest.xml` (Common Controls 6.0.0.0, publicKeyToken `6595b64144ccf1df`).
- tauri-build reads `STATIC_VCRUNTIME` from the build script's own environment (`lib.rs:708`), so `std::env::set_var` in `build.rs` before `tauri_build::try_build` takes effect; Cargo's `rerun-if-env-changed` looks at the external env, so no rebuild loop. Only the `msvc` target env is affected.
- The gnu cross-check compiles `build.rs` with `CARGO_CFG_TARGET_ENV=gnu`; gate all MSVC-only link args on `CARGO_CFG_TARGET_ENV == "msvc"`.
- GitHub `windows-latest` runners have an interactive desktop and WebView2; Git Bash provides `timeout`, `grep`, `sha256sum`. The debug `voxy.exe` is a console-subsystem binary (`windows_subsystem` only in release), so `VOXY_LOG_STDERR=1` reaches a redirected stderr. Log tokens (contract C2): `хук установлен`, `startup complete`.
- NSIS (Tauri template, `currentUser`): `-setup.exe /S` silent, `/P` passive, both kill a running `Voxy.exe`; installs to `%LOCALAPPDATA%\Voxy\Voxy.exe` (`MAINBINARYNAME` = product name); registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Voxy` with `UninstallString` (`"<dir>\uninstall.exe"`), `DisplayIcon`, `DisplayVersion`, `InstallLocation`; `uninstall.exe /S` silent. Autostart writes `HKCU\…\CurrentVersion\Run\Voxy`. Keyring entries: generic credentials `api-key:openai.com.dizraid.voice`, `api-key:groq.com.dizraid.voice`, `api-key:elevenlabs.com.dizraid.voice`. App data after WS-C: `%LOCALAPPDATA%\com.dizraid.voice` (also remove a legacy `%APPDATA%\com.dizraid.voice` if present). GitHub release assets expose `digest` (`sha256:…`) in the API.
- `.gitattributes` forces CRLF for `*.ps1`; keep `install.ps1` ASCII-only (PowerShell 5.1 misreads UTF-8 without BOM).

**Tasks**
1. **`src-tauri/windows/tests.manifest`** — copy of tauri-build's default manifest (XML header + Common Controls v6 dependency), with a comment explaining it is for test binaries only.
2. **`build.rs`** — add `cargo:rerun-if-changed=windows/tests.manifest`; when `CARGO_CFG_TARGET_OS == "windows" && CARGO_CFG_TARGET_ENV == "msvc"`: (a) if `STATIC_VCRUNTIME` is unset, `std::env::set_var("STATIC_VCRUNTIME", "true")` with a comment (sherpa-onnx's Windows libraries are /MT; CLI 2.11 sets it, plain cargo does not); (b) `println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED")` and `println!("cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}", <absolute path from CARGO_MANIFEST_DIR>/windows/tests.manifest)` with a comment citing tauri-apps/tauri#13419. Keep everything else.
3. **`ci.yml`** — Windows matrix entry: `test_args: ""`, update comments (L59, L107–113), timeout stays 45. Add after "Unit tests" an **experimental, non-blocking** step (`if: runner.os == 'Windows'`, `continue-on-error: true`, `shell: bash`, env `VOXY_LOG_STDERR=1`, `RUST_LOG=info`): run `timeout 25 ./target/debug/voxy.exe >smoke.log 2>&1 || true` from `src-tauri`, print the log, then `grep -q "хук установлен" smoke.log && grep -q "startup complete" smoke.log`. Note in a comment that the first run starts the default-model download and is killed after 25 s (harmless on a throwaway runner). Update the Summary step to say whether tests ran and the smoke outcome (`steps.<id>.outcome`). Add `install.ps1` to the `paths` filters and a small job `installer-script` on `windows-latest` (`if: github.event_name != 'pull_request'`, `continue-on-error: true` initially) that parses the script (`[System.Management.Automation.Language.Parser]::ParseFile`) and runs `install.ps1 -DryRun`-style checks you design (e.g. `-Uninstall -KeepData` against a machine without Voxy must exit 0 and print "not installed"). Keep CI cheap: no release-profile builds.
4. **`release.yml`** — make Windows required: remove `continue-on-error` from the Windows artifact download; in "Check the macOS app" (rename to "Check the bundles"), fail if `Voxy_${VERSION}_x64-setup.exe` is missing, smaller than 10 MB, or does not start with the `MZ` signature (`head -c2`); keep all macOS checks. Update the header comment (L26–28) and the release notes: a first-class "Windows 10/11 (x64)" section with the one-liner `irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1 | iex`, the SmartScreen note (unsigned), where Voxy lives after install (tray, Start menu), and the uninstall one-liner. Keep the attestation step covering all files (it already does).
5. **`install.ps1`** — Windows counterpart of `install.sh`, deliberately small (~250 lines), PowerShell 5.1-compatible, all code in functions with `Main` at the end, `$ErrorActionPreference = 'Stop'`, `Set-StrictMode -Version 2`, TLS 1.2 enforced. Usage header:
   - install/update: `irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1 | iex`
   - with options: `& ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -Uninstall [-KeepData]`
   - `param([switch]$Uninstall, [switch]$KeepData, [switch]$NoLaunch)`; env knobs `VOXY_INSTALLER` (local `-setup.exe` instead of downloading), `GITHUB_TOKEN` (API header only), `NO_COLOR`.
   - Install: require Windows (`$env:OS -eq 'Windows_NT'`), 64-bit, warn on ARM64 (`$env:PROCESSOR_ARCHITECTURE -eq 'ARM64'`: x64 emulation, slower), warn below build 17763; `Invoke-RestMethod` `releases/latest` with `User-Agent`, tag must match `^v\d+\.\d+\.\d+$`, pick the asset ending in `_x64-setup.exe`, download to a fresh temp dir (`$ProgressPreference='SilentlyContinue'`), verify `Get-FileHash` against the asset `digest` when present (fail on mismatch, warn when absent), stop running `Voxy` processes (`Get-Process Voxy -ErrorAction SilentlyContinue | Stop-Process`), run `Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru` and fail on non-zero exit, locate the installed exe from the Uninstall key (`InstallLocation` or `DisplayIcon`) with `%LOCALAPPDATA%\Voxy\Voxy.exe` as fallback, launch unless `-NoLaunch`, print where data and logs live.
   - Uninstall: read `UninstallString`; if missing say so and continue to data cleanup; stop Voxy; run `uninstall.exe /S` and wait; unless `-KeepData` remove `%LOCALAPPDATA%\com.dizraid.voice`, `%APPDATA%\com.dizraid.voice`, legacy `%LOCALAPPDATA%\Voxy.log`, the three Credential Manager entries via `cmdkey /delete:<target>` (ignore failures), and the `Run`/`StartupApproved\Run` values named `Voxy`. Idempotent and safe to re-run.
   - Never download over plain http; never run anything from the current directory; print each step like `install.sh` (`==>` style, colors off when `NO_COLOR` or not a TTY).
   - Verification here: `pwsh` is not installed; if you can fetch the PowerShell Linux tarball from GitHub into a temporary folder, run `pwsh -NoProfile -Command '[System.Management.Automation.Language.Parser]::ParseFile("install.ps1",[ref]$null,[ref]$e); $e'` to parse-check; otherwise review carefully and say so in the report.
6. **`install.sh:237`** — the non-macOS message points to `install.ps1` (one-liner) instead of the raw `-setup.exe`.

**Do not**
- Touch `.github/scripts/fetch-sherpa-onnx.sh`, `sherpa-onnx.sha256`, Dependabot config, `tauri.conf.json`, `Cargo.*`, or any Rust file but `build.rs`.
- Make the smoke run or the installer-script job blocking in this pass (they are the first real runtime signals; the orchestrator flips them to blocking after they are green on `main`).
- Add release-profile or `tauri build` steps to `ci.yml`.

**Acceptance**
- Both cross-check scripts pass (`build.rs` compiles for gnu and emits no MSVC flags there — check with `scripts/cross-check.sh win check -vv 2>&1 | grep -c MANIFEST` → 0).
- `python3 - <<'EOF'` / `yamllint`-free sanity: `python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in ['.github/workflows/ci.yml','.github/workflows/release.yml']]"` passes.
- Report includes: the exact CI step names/ids, what a human must check on the first green `main` run (test count on Windows, smoke log excerpt), and the `install.ps1` option matrix for the README.

---

## 5. Phase 2 — integration (orchestrator)

1. **Merge order and checks.** Apply the five trees, then run all four cross-check commands plus `node --check` on the three JS files. Resolve any `log::` message drift against contract C2 and the backdrop query against C1 (grep `backdrop=` in `lib.rs` and `boot.js`).
2. **Docs** (all paths relative to the repo):
   - `README.md`: tagline → "for macOS and Windows"; badges add "Windows 10/11 x64"; Install gets a "Windows" subsection (PowerShell one-liner, SmartScreen, uninstall, requirements: Windows 10 1809+/11, x64, WebView2 is installed by the installer if missing); "Why Voxy" → "stays on your computer"; Usage mentions Right Ctrl on Windows and left-click on the tray; rewrite the `## Windows` section (first release for Windows, what is different, known limits: unsigned, UIPI, Win key); Roadmap: replace "Windows: testing" with "Windows code signing / winget".
   - `BUILD_WINDOWS.md`: new status header (built and unit-tested in CI; smoke-run; report issues), version-agnostic file names, first-launch section (Settings opens on first run, tray left click, data under `%LOCALAPPDATA%\com.dizraid.voice\`, log path), "Differences from macOS" (dark native title bar, Mica on Win11, opaque on Win10; clipboard restore semantics; sounds), "Known gaps" → UIPI (with the new message), Win key/Left Alt caveats, unsigned installer; keep the testing checklist and extend it with §6.2 below; keep the CUDA note.
   - `docs/DEVELOPMENT.md`: project layout (`windows/tests.manifest`, `install.ps1`), CI paragraph (tests now run on Windows; smoke run; installer-script job), "Platform-specific code" (new Windows items; `windows-version`), a new "Cross-checking Windows and macOS from Linux" subsection (see CLAUDE.md content below), release section (Windows required, `install.ps1` picks `_x64-setup.exe`, env knobs).
   - `docs/TROUBLESHOOTING.md`: logs (new Windows path), island messages (add the UIPI message), "The hotkey does nothing" (Windows: Win key, elevated apps, check the log for `хук установлен`/re-hook errors), data locations table (LOCALAPPDATA), uninstalling (PowerShell one-liner, `cmdkey` targets, Run key), microphone privacy settings path (`ms-settings:privacy-microphone`).
   - `SECURITY.md`: scope now includes `install.ps1`; verifying a download lists the `.exe`.
   - `.github/ISSUE_TEMPLATE/bug_report.md`: Windows log path.
   - Save this plan as `docs/WINDOWS_PORT_PLAN.md` and link it from `BUILD_WINDOWS.md`.
3. **`CLAUDE.md`** (new, repo root) — contents:
   - What Voxy is, platform matrix, where the code lives (`src-tauri/src/*` one-liners as in DEVELOPMENT.md; `ui/*`; workflows; `install.sh`/`install.ps1`).
   - Build and test commands per platform (`cargo tauri dev/build`, `cargo test --lib`, `VOXY_LOG_STDERR=1`, `model_smoke`), the UI preview (`python3 -m http.server 4173 --directory ui`, `?platform=win&backdrop=mica`).
   - **Cross-check from Linux** (no webkit2gtk, nothing links): `scripts/cross-check.sh win|mac check|clippy --all-targets` (one-time `pip install ziglang`; the script adds the rustup targets, writes zig wrappers and a windres stub to `~/.cache/voxy-cross`, and shares `src-tauri/target/xwin`/`xmac` across worktrees). State that the gnu target validates the same `cfg(windows)` code as MSVC releases but not MSVC-only linker flags, and that `test --no-run` cannot link here.
   - Conventions: `#[cfg]` only at the OS-call layer, pure testable decision functions; no logging/locks in hook or audio callbacks; IPC ACL (`app_commands.rs` + capabilities + `ipc_is_scoped_per_window`); strict CSP (no inline scripts/styles); `Cargo.lock` committed, `--locked` builds, update `sherpa-onnx.sha256` with `sherpa-onnx-sys` bumps; `tauri.conf.json` window config is macOS-shaped and Windows derives its variant in `show_settings`; data lives under the identifier folder; log paths; Russian comments/log lines are fine, docs and UI are English; no emojis.
   - "Before you finish": the four cross-check commands, `node --check`, keep the two pre-existing clippy warnings as the only ones.
4. **Consistency pass.** Grep for `%APPDATA%\com.dizraid.voice`, `%LOCALAPPDATA%\Voxy.log`, "experimental", "has not been run on real hardware", `Voxy_0.1.0` across `*.md`, `install.sh`, workflows; make the `README` Windows section and the release notes agree with `install.ps1`'s option names.
5. **Final checks.** All four cross-check commands; `git status` shows only intended files; no files under `src-tauri/target`, `src-tauri/gen`, or `permissions/autogenerated` staged; `python3 -m json.tool` on `tauri.conf.json`; YAML loads. Then hand over for commit (the owner decides on branch/commit), with the manual checklist below attached to the PR/commit message.

---

## 6. Risks and manual test checklist

### 6.1 Risks (what could still be wrong without hardware)

| Risk | Why it is plausible | Mitigation in the plan / what to watch |
|---|---|---|
| Test manifest trick does not fix `0xC0000139` | Unverified that only the comctl32 v6 import is missing | CI tells immediately; fallback: restore `--no-run` and open an issue (WS-E documents the switch). |
| Mica window shows see-through on some Win11 builds (22000–22522 use the undocumented attribute) | window-vibrancy's older path | Gate stays; if reports come in, raise the threshold to 22621. |
| `transparent: true` + decorated window on Win11 has odd shadow/border | Known-good combination in Tauri examples, but not with `MicaDark` + dark theme + non-resizable | Checklist item; fallback is the opaque path (one-line change). |
| Periodic re-hook loses a key event or fights other hooks | Unhook/hook is microseconds; only when no key is held | Interval 60 s, debug logging; can be disabled by setting the interval to 0 if reports come in. |
| `THREAD_PRIORITY_HIGHEST` on the hook thread is not enough under load | Inference threads at normal priority | If the hotkey still dies under load, consider `TIME_CRITICAL` or lowering sherpa thread count on Windows. |
| Island click-through toggling (`WS_EX_TRANSPARENT`) with WebView2 | Tauri supports it, but the Cancel button must receive clicks without activating the window | Checklist; `focusable(false)` keeps the user's text field focused. |
| `SetForegroundWindow` refused for the second-instance path | Windows foreground lock; tao injects an Alt tap as a workaround | Documented caveat (Left Alt as hotkey); Settings may only flash in the taskbar — acceptable. |
| `cmdkey` target names differ | keyring's `windows-native` store builds `user.service`; verified from the store's docs, not from a machine | `install.ps1` ignores delete failures and prints remaining entries (`cmdkey /list`). |
| GitHub Windows runner cannot install a `WH_KEYBOARD_LL` hook or start WebView2 | Interactive session should exist | Smoke step is non-blocking until proven. |
| DPI/multi-monitor placement of the island | Physical coordinates from the primary monitor's work area; no reaction to display changes at runtime | Checklist; follow-up: re-center on `ScaleFactorChanged`/display change. |
| WebView2 missing on a locked-down PC and the bootstrapper download blocked | `downloadBootstrapper` needs internet | Documented; switch to `embedBootstrapper` (+1.8 MB) if reports come in. |
| Roaming → local data dir | No Windows users exist yet; macOS path identical | None needed; mention in release notes anyway. |

### 6.2 Manual test checklist (human with a Windows PC; attach `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` to reports)

Install and first run
1. `irm …/install.ps1 | iex` on Windows 11 and on Windows 10: downloads, verifies, installs silently, launches. SmartScreen "More info → Run anyway" only when running the `.exe` by hand.
2. First launch opens the Settings window (dark title bar, Mica glass on Win11, opaque on Win10, no white flash, window centered, not resizable, close keeps Voxy in the tray).
3. Tray: colored icon visible on dark and light taskbars; left click and double click open Settings; right click shows "Settings…/Quit Voxy"; tooltip.
4. Model download progress visible; download survives closing Settings; re-opening Settings shows status.

Hotkey and dictation
5. Notepad: hold Right Ctrl, speak, release → text appears; island slides down from the top center while recording, shows "Transcribing…", disappears. Timer colors near the limit.
6. Same in Word, Chrome/Edge address bar and a web form, Windows Terminal, VS Code, Telegram/Slack desktop.
7. Russian (or any non-Latin) keyboard layout active: pasting still works.
8. Toggle and Dynamic modes; minimum duration ignores taps; maximum length auto-stops.
9. Change hotkey to F8, Caps Lock, Right Alt; each works; Esc cancels capture; capture ignores keys when Settings loses focus; choosing Right Win shows the warning and a tap opens Start (expected).
10. Press F5/Ctrl+P/Ctrl+F while Settings is focused: nothing reloads/prints/finds. Right click shows no browser menu.
11. Hold the hotkey while a long transcription runs (CPU busy): the hotkey still works afterwards; after 10 minutes idle and after sleep/resume it still works; log shows no re-hook errors.
12. Island: click it while recording → Cancel appears and works; the text field in the other app keeps focus throughout; when idle the island never blocks clicks at the top of the screen.

Clipboard
13. Copy "A", dictate "B" → "B" pasted, clipboard holds "A" again after ~0.5 s; Win+V history does not list "B".
14. Copy a password from 1Password/Bitwarden/KeePassXC, dictate → the password is not put back (clipboard holds the dictated text, then stays as the manager leaves it).
15. Copy something during the 0.5 s window (quick Ctrl+C in another app) → your copy is kept, not overwritten.
16. Notepad "Run as administrator": dictation shows "Paste blocked by an admin app — text kept in clipboard"; Ctrl+V pastes manually; History has the text.

Settings and system integration
17. Save/verify/delete an API key (OpenAI/Groq/ElevenLabs); `cmdkey /list` shows `api-key:<provider>.com.dizraid.voice`; online dictation works.
18. "Start with Windows" on/off → Task Manager → Startup apps; after sign-in Voxy starts in the tray without a window (no island flash).
19. Sounds follow the Windows sound scheme; "No Sounds" scheme → silence, no default beep.
20. Microphone selection lists devices; a mic with an unusual format (USB mic, Bluetooth headset) records; Windows Privacy → Microphone off → "Microphone unavailable".
21. Multi-monitor with different DPI (100 % + 150 %), taskbar docked at the top: island centered on the primary monitor below the taskbar; Settings text crisp at 125/150/200 %.
22. Second launch (Start menu) brings up the running copy's Settings; only one `Voxy.exe` in Task Manager.
23. Update: re-run the one-liner while Voxy runs → it is stopped, replaced, relaunched; settings/history/models kept.
24. Uninstall via Settings → Apps and via `-Uninstall`; with `-KeepData` the data folder stays; without it `%LOCALAPPDATA%\com.dizraid.voice`, credentials and the Run key are gone.
25. Log: `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` contains "startup complete", "хук установлен", and no repeated errors during an idle hour.

---

### Critical Files for Implementation
- /home/user/voxy/src-tauri/src/lib.rs
- /home/user/voxy/src-tauri/src/hotkey.rs
- /home/user/voxy/src-tauri/src/platform.rs
- /home/user/voxy/src-tauri/tauri.conf.json
- /home/user/voxy/src-tauri/build.rs

---

## 7. Implementation notes (what was actually done)

The five workstreams were implemented in parallel, each in its own git worktree, and merged into one branch; the docs pass (§5) followed. Deviations from the briefs above, all deliberate:

- **C5, island at the bottom (WS-C, WS-D).** Requested by the repo owner during implementation: on Windows the island is a floating pill bottom-center above the taskbar (window bottom edge on the primary monitor's work-area bottom; the page anchors the pill with a 12 px gap, rises in, sinks out, opacity-only under reduced motion). `island::reposition` re-applies the placement at every recording start, so taskbar/resolution/scale changes are picked up. Long error messages size the pill to their text (up to 480 px on Windows; on macOS the error pill widens only when the text would be clipped).
- **WS-A.** The re-hook waits while keys are held, but at most 10 ticks (`MAX_REHOOK_DEFERRALS`): key-ups on the secure desktop (Win+L, Ctrl+Alt+Del, UAC) are never seen, and a stuck bit would otherwise block self-healing for the whole session.
- **WS-B.** The elevated-foreground check runs **before** sending Ctrl+V, not only after a failed send: UIPI may drop the input while `SendInput` reports success, and the clipboard restore would then overwrite the unread text. `foreground_is_elevated` is true only when Voxy itself is not elevated.
- **WS-C.** Settings re-open while the hidden window is still loading waits for the page load, with a 3 s fallback (`SETTINGS_LOAD_GRACE`); a minimized Settings window is restored from the tray.
- **WS-D.** New `ui/guard.js` (shared accelerator/context-menu guard; the context menu stays available in inputs and on selected text). Island enter/exit uses `@starting-style` and `transition-behavior: allow-discrete` (Chromium 117+, i.e. any current WebView2).
- **WS-E, test manifest.** `cargo:rustc-link-arg-tests` reaches integration tests only, not `cargo test --lib`, so the manifest is `src-tauri/windows/app.manifest`, embedded by link.exe (`/MANIFEST:EMBED` + `/MANIFESTINPUT`) into **every** MSVC binary, and tauri-build is told not to embed its own copy (`WindowsAttributes::new_without_app_manifest()`; two `RT_MANIFEST` resources would fail with CVT1100). The exe also gets link.exe's default `asInvoker` UAC section.
- **WS-E, release.** The installer size floor is 5 MB (estimate 6–8 MB; raise it after the first real release). The publish job runs only when every build succeeded. `ci.yml` no longer sets `STATIC_VCRUNTIME` globally, so CI exercises the `build.rs` default.
- **WS-E, install.ps1.** ~490 lines; waits on the installer process itself, runs the NSIS uninstaller with `_?=` so the wait covers the real uninstall, warns and does not launch Voxy when PowerShell runs elevated, exit code 2 for conflicting options.

Verification available here was compile-level only (`scripts/cross-check.sh` for both targets, `node --check`, YAML loading, PowerShell 7 on Linux with mocks for `install.ps1`, Playwright screenshots of the UI preview). Everything in §6.2 still needs a real Windows PC.
