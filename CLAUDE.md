# CLAUDE.md

Notes for Claude Code (and other agents) working in this repository. Human-facing docs are in `README.md`, `docs/DEVELOPMENT.md`, `docs/TROUBLESHOOTING.md` and `BUILD_WINDOWS.md`; keep them in sync when behavior changes.

## What Voxy is

Push-to-talk dictation: hold a hotkey, speak, release, and the recognized text is pasted at the cursor. Recognition runs on-device (sherpa-onnx, statically linked) or through online providers (OpenAI, Groq, ElevenLabs). Tauri 2 + Rust backend in `src-tauri/`, plain static HTML/CSS/JS UI in `ui/` (no npm, no bundler).

| Platform | Status |
|---|---|
| macOS 14+, Apple Silicon | Primary platform; built, tested and released |
| Windows 10/11, x64 (MSVC) | Port; built and unit-tested in CI, released as an NSIS installer; see `BUILD_WINDOWS.md` and `docs/WINDOWS_PORT_PLAN.md` |
| Linux | Not a target (no native build is possible here: Tauri needs webkit2gtk) |

## Where things live

- `src-tauri/src/lib.rs`: app setup, Tauri commands, tray, Settings window (`show_settings`; the Windows variant is derived at runtime from the macOS-shaped config in `tauri.conf.json`), logging, unit tests that parse `tauri.conf.json`/capabilities.
- `hotkey.rs`: global hotkey (CGEventTap on macOS, `WH_KEYBOARD_LL` on Windows) and the Hold/Toggle/Dynamic state machine.
- `audio.rs` (cpal capture), `worker.rs` (recognition thread, paste, history), `asr.rs` (sherpa-onnx), `models.rs` + `catalog.json` (model catalog, downloads), `online.rs` (providers, keys in the OS keychain/Credential Manager), `discovery.rs`.
- `island.rs`: the island window. macOS: NSPanel at the camera notch. Windows: transparent always-on-top window, pill bottom-center above the taskbar.
- `output.rs` (clipboard + synthetic Ctrl+V/Cmd+V), `platform.rs` (platform seam: sounds, permissions, clipboard generation/secret detection, elevation check), `store.rs` (settings/history JSON, atomic writes).
- `app_commands.rs` + `capabilities/*.json`: per-window IPC ACL.
- `ui/`: `settings.*` (Settings window), `island.*` (island), `boot.js` (platform/backdrop attributes before first paint).
- `.github/workflows/ci.yml` (debug build + unit tests on macOS and Windows), `release.yml` (tag `vX.Y.Z` → bundles → GitHub release). `.github/scripts/fetch-sherpa-onnx.sh` + `src-tauri/sherpa-onnx.sha256` pin the native libraries.
- `install.sh` (macOS one-command installer), `install.ps1` (Windows one-command installer).
- `scripts/cross-check.sh`: cross type-check for Windows/macOS from Linux (see below).

## Building and testing (on the real OS)

All from `src-tauri/`:

```sh
cargo tauri dev                    # run (VOXY_LOG_STDERR=1 to log to the terminal)
cargo tauri build                  # release bundle (.app/.dmg on macOS, NSIS -setup.exe on Windows)
cargo test --lib                   # unit tests (network tests are #[ignore])
cargo run --release --example model_smoke -- <model-id> audio.wav <models-dir>
```

UI preview without Tauri (from the repo root): `python3 -m http.server 4173 --directory ui`, then `settings.html` / `island.html`. Demo knobs: `?tab=…`, `?perm=0`, `?platform=win|mac`, `?backdrop=mica|opaque`; `window.__settingsDebug`, `window.__islandDebug` in the console.

## Cross-checking from Linux (cloud sessions)

No macOS or Windows machine is available in cloud sessions, so the only compile-level verification is:

```sh
pip install ziglang                 # once (add --break-system-packages if pip refuses)
scripts/cross-check.sh win check --all-targets
scripts/cross-check.sh win clippy --all-targets
scripts/cross-check.sh mac check --all-targets
scripts/cross-check.sh mac clippy --all-targets
```

- Windows is checked as `x86_64-pc-windows-gnu` (same `cfg(windows)` Rust code as the MSVC release target, but MSVC-only linker flags in `build.rs` are not exercised); macOS as `aarch64-apple-darwin`.
- Nothing links, so `cargo test` cannot run here; unit tests run in CI on both OSes. Put decision logic in `#[cfg]`-free pure functions so it is testable everywhere.
- Build output goes to `src-tauri/target/xwin` / `xmac` of the main checkout and is shared by git worktrees.
- Pre-existing clippy warnings (`manual_checked_ops` in `models.rs` and `examples/model_smoke.rs`) are the baseline; don't add new ones.
- JS: `node --check ui/<file>.js`. JSON: `python3 -m json.tool`. Workflows: `python3 -c "import yaml; yaml.safe_load(open(path))"`.
- UI can be checked visually with the preinstalled Playwright + Chromium against the preview server.

## Conventions

- `#[cfg(target_os = …)]` only at the thin OS-call layer; keep shared logic platform-free.
- Never log, allocate strings or take locks inside a Win32 hook callback, the macOS event tap callback or the cpal audio callback.
- Every `unsafe` block gets a `// SAFETY:` comment where the surrounding code does so.
- IPC: a new `#[tauri::command]` must be listed in `app_commands.rs` and granted in the right `capabilities/*.json`; the test `ipc_is_scoped_per_window` enforces it.
- CSP is strict: no inline `<script>`, no `style="…"` attributes, no remote URLs; set styles via CSSOM. Test `csp_is_strict` enforces it.
- `Cargo.lock` is committed and builds use `--locked`. When `sherpa-onnx-sys` changes, update `src-tauri/sherpa-onnx.sha256` in the same change.
- App data lives under the bundle identifier folder `com.dizraid.voice` (macOS: `~/Library/Application Support/`, Windows: `%LOCALAPPDATA%\`). Log: `~/Library/Logs/Voxy.log` on macOS, `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` on Windows.
- Code comments and log lines are often in Russian; that is fine. Docs, UI strings, commit messages: English. No emojis.
- Workflows: third-party actions pinned to full commit SHAs, read-only tokens in build jobs, `persist-credentials: false`.
- `install.ps1` is ASCII-only (PowerShell 5.1) and CRLF (`.gitattributes`).
