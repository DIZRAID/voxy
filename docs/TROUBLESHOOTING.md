# Troubleshooting

[Back to the README](../README.md)

- [Logs](#logs)
- [Messages on the island](#messages-on-the-island)
- [Windows protected your PC (SmartScreen)](#windows-protected-your-pc-smartscreen)
- [The hotkey does nothing](#the-hotkey-does-nothing)
- [Text is recognized but not pasted](#text-is-recognized-but-not-pasted)
- [The clipboard after a dictation](#the-clipboard-after-a-dictation)
- [Every recording ends with "Didn't catch that"](#every-recording-ends-with-didnt-catch-that)
- [The menu bar icon is missing](#the-menu-bar-icon-is-missing)
- [A model download fails](#a-model-download-fails)
- [The build fails while downloading sherpa-onnx](#the-build-fails-while-downloading-sherpa-onnx)
- [Data locations](#data-locations)
- [Uninstalling Voxy](#uninstalling-voxy)

## Logs

Logs are written to `~/Library/Logs/Voxy.log` on macOS and to `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` on Windows (paste that path into the File Explorer address bar to open it). Set `VOXY_LOG_STDERR=1` to send them to stderr instead; on Windows that helps only with a debug build started from a terminal, because the installed app has no console. `RUST_LOG` overrides the default `info` level, except for the HTTP client (`ureq`), which never logs below `warn` because its debug output contains request headers (API keys). Before you attach a log to a public issue, read it and remove anything private. Log messages, model download errors shown in Settings → Model, and the `model_smoke` output are currently in Russian.

For a debug log on Windows, quit Voxy (tray icon → Quit Voxy) and start it from PowerShell with `RUST_LOG` set. It logs at that level until you quit it:

```powershell
$env:RUST_LOG = "debug"
& "$env:LOCALAPPDATA\Voxy\Voxy.exe"
```

## Messages on the island

| Message | Meaning |
|---|---|
| Model is not ready yet | Shown when you press the hotkey: the active model is not downloaded or failed to load. Check Settings → Model |
| Model is not ready | Shown after a recording: the local model failed to load, so the recording was not transcribed. Check Settings → Model and the [log](#logs) |
| Online engine unavailable | The selected online provider has no saved API key, or macOS denied Voxy access to the key in the Keychain (this can happen after an update or a rebuild). Add the key again in Settings → Model, or choose Always Allow when macOS asks |
| Microphone unavailable | The input device could not be opened. Check the device in Settings → General → Microphone; on Windows, also the [microphone privacy settings](#every-recording-ends-with-didnt-catch-that) |
| Didn't catch that | No speech was recognized |
| Paste failed — text kept in clipboard | The synthetic ⌘V (Ctrl+V on Windows) could not be sent, for example because Accessibility was turned off while Voxy was running. Paste with ⌘V or Ctrl+V yourself; the text is also in History |
| Paste blocked by an admin app — text kept in clipboard | Windows: the active window belongs to an app that runs as administrator and Voxy does not, so Windows would block the synthetic Ctrl+V. Voxy did not send it: press Ctrl+V yourself. The text is also in History. See [Text is recognized but not pasted](#text-is-recognized-but-not-pasted) |
| `<Provider>: invalid API key` / `rate limit or no credits` / `No connection to <Provider>` | An error from the online provider |
| `<Provider>: server error <code>` | The provider returned a server error. Try again later; the details are in the [log](#logs) |
| Something went wrong | An internal error; the recording was discarded. See the [log](#logs) |

## Windows protected your PC (SmartScreen)

The Windows installer is not code-signed yet. When you run a `_x64-setup.exe` that you downloaded with a browser, Microsoft Defender SmartScreen may say "Windows protected your PC". Click **More info**, then **Run anyway**.

The PowerShell install command in the [README](../README.md#install) downloads the installer itself and checks its SHA-256 against the release, and it does not get this warning. To check a file you downloaded by hand, see [SECURITY.md](../SECURITY.md).

## The hotkey does nothing

- Make sure you are pressing the key shown in Settings → General → Hotkey.
- After a reinstall, make sure the old copy is not still running.

On macOS:

- Check the Settings window for the permissions banner: both Accessibility and Input Monitoring must be granted.
- If you have updated or rebuilt the app, the old permission entries may no longer match it (see [After an update](../README.md#after-an-update)). Reset them; this also restarts Voxy, and macOS asks for both permissions again:

  ```sh
  curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --reset-permissions
  ```

  Or quit Voxy, reset them yourself and open the new version:

  ```sh
  tccutil reset Accessibility com.dizraid.voice
  tccutil reset ListenEvent com.dizraid.voice
  ```

- If you started Voxy with `cargo tauri dev`, grant the permissions to your terminal app.
- Check the log. `accessibility=true input_monitoring=true`, followed by a line that contains `event tap`, means the hotkey is active:

  ```sh
  grep -E 'accessibility=|event tap' ~/Library/Logs/Voxy.log | tail
  ```

On Windows (no permissions are needed there):

- **Apps that run as administrator.** While a window of an app that runs as administrator has focus (a terminal opened with "Run as administrator", Registry Editor and so on), Windows keeps apps without administrator rights from seeing the keys you press in it (User Interface Privilege Isolation). So the hotkey most likely does nothing there. Click into a normal window to dictate. Pasting into such apps is blocked as well (see [Text is recognized but not pasted](#text-is-recognized-but-not-pasted)).
- **Win and Left Alt.** Tapping a Win key alone opens the Start menu, so the Win keys make poor hotkeys (Settings warns about it). Left Alt opens the menus of many apps, and Voxy itself sends a short Left Alt tap when it brings its Settings window to the front (a Windows workaround), which in Toggle or Dynamic mode can start or stop a recording. Right Ctrl (the default) or an F-key works best.
- **Check the log.** A line that contains `хук установлен` means the hotkey is active. In PowerShell:

  ```powershell
  Select-String -Encoding UTF8 -Path "$env:LOCALAPPDATA\com.dizraid.voice\Voxy.log" -Pattern 'хук' | Select-Object -Last 10
  ```

The keyboard hook's log lines, with English translations:

| Log line | Meaning |
|---|---|
| `глобальный хоткей активен (клавиатурный хук установлен)` | "Global hotkey active (keyboard hook installed)". The hotkey works. Logged at startup, and again after the hook had to be reinstalled |
| `клавиатурный хук был снят системой, ставлю заново` | "The system removed the keyboard hook, reinstalling it". Windows removes a hook that answers too slowly, usually under heavy load; Voxy notices, normally within a minute, and installs it again. Harmless now and then; if it happens often, please report it |
| `не удалось установить клавиатурный хук (GetLastError=…)` | "Could not install the keyboard hook". The hotkey does not work; Voxy tries again every minute |
| `не удалось снять клавиатурный хук (GetLastError=…), повтор через минуту` | "Could not remove the keyboard hook, retrying in a minute". The old hook stays in place meanwhile |
| `цикл сообщений хука завершён (…): хоткей выключен до перезапуска` | "The hook's message loop ended: the hotkey is off until a restart". Quit Voxy, start it again and please report it |
| `не удалось запустить поток клавиатурного хука: …` | "Could not start the keyboard hook thread". The hotkey does not work until Voxy restarts |
| `не удалось поднять приоритет потока хука (GetLastError=…)` | "Could not raise the hook thread's priority". The hotkey works, but Windows is more likely to remove the hook under load |
| `таймер переустановки хука не создан (GetLastError=…): самолечение выключено` | "Re-hook timer not created: self-healing is off". If Windows removes the hook, the hotkey stays off until Voxy restarts |

## Text is recognized but not pasted

If the island says "Paste failed — text kept in clipboard", the text is in the clipboard: paste it with ⌘V (Ctrl+V on Windows). On a Mac, check that Accessibility is still granted.

If it says "Paste blocked by an admin app — text kept in clipboard" (Windows), the active window belongs to an app that runs as administrator, and Windows does not let an app without administrator rights type into it. Voxy checks this before it sends Ctrl+V and leaves the text in the clipboard, so press Ctrl+V yourself. To paste into such apps directly, Voxy would have to run as administrator as well. With `RUST_LOG=debug` (see [Logs](#logs)), the log shows this check in lines that start with `uipi:`.

Otherwise the ⌘V or Ctrl+V was sent but the app did not accept it (for example, no text field had focus). The clipboard has already been restored to what it held before, so copy the text from Settings → History.

## The clipboard after a dictation

Voxy pastes through the clipboard: it puts the text there, sends ⌘V (Ctrl+V on Windows) and about half a second later puts back what the clipboard held before. This works the same on both systems, with a few exceptions:

- If you copy something yourself during that half second, Voxy keeps your copy and does not put the old content back.
- If the clipboard held a password from a password manager, Voxy does not put it back, and the clipboard keeps the dictated text. Password managers such as 1Password, Bitwarden and KeePassXC mark what they copy (on macOS as concealed or transient content, on Windows with the `ExcludeClipboardContentFromMonitorProcessing` format). Voxy could only put it back without that mark, so it leaves it out.
- If the clipboard held something other than text (an image, files), it is not put back either.
- If the paste could not be sent ("Paste failed" or "Paste blocked by an admin app"), the dictated text stays in the clipboard.

The dictated text and the restored content are marked to stay out of clipboard history (Win+V on Windows; clipboard managers such as Maccy, Raycast or Paste on macOS).

## Every recording ends with "Didn't catch that"

On macOS, check System Settings → Privacy & Security → Microphone: Voxy must be turned on there (with `cargo tauri dev`, your terminal app).

On Windows, open Settings → Privacy & security → Microphone (on Windows 10, Settings → Privacy → Microphone) and make sure microphone access is on, including "Let desktop apps access your microphone": Voxy is a desktop app, so it has no switch of its own there. To open the page directly, press Win+R and run `ms-settings:privacy-microphone`.

Also check the input device in Settings → General → Microphone. Each recording logs the device and format Voxy opened, in a line like `микрофон: Microphone (USB Audio), 48000 Гц, каналов 1, формат F32` ("microphone: device, sample rate in Hz, channels, sample format"). If it names the wrong device, choose the right one in Settings.

## The menu bar icon is missing

On a MacBook with a notch, a full menu bar can hide Voxy's icon. Open Voxy again from `/Applications`: since only one copy runs at a time, this brings up the Settings window of the running copy.

On Windows, the icon is in the notification area at the right end of the taskbar, and Windows 11 may put it in the overflow under the **^** arrow; drag it onto the taskbar to keep it in view. Left-click it to open Settings, right-click it for the menu. Starting Voxy again from the Start menu also opens the Settings window of the running copy.

## A model download fails

The error appears under the model in Settings → Model. Click Get again to resume from where it stopped. If a file fails its checksum, it is deleted and must be downloaded again.

Voxy downloads models directly from huggingface.co (Parakeet TDT 110M from github.com) and does not use the macOS proxy settings. If those sites are blocked on your network, use a system-wide VPN.

## The build fails while downloading sherpa-onnx

The build script of the `sherpa-onnx-sys` crate downloads a prebuilt static library from GitHub. The build honors `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`.

If GitHub is still unreachable, download the archive yourself and set `SHERPA_ONNX_ARCHIVE_DIR` to the folder that contains it. The file name must match exactly:

- `sherpa-onnx-v<version>-osx-arm64-static-lib.tar.bz2` on Apple Silicon
- `sherpa-onnx-v<version>-osx-x64-static-lib.tar.bz2` on Intel Macs (untested)
- `sherpa-onnx-v<version>-win-x64-static-MT-Release-lib.tar.bz2` on Windows x64

`<version>` is the `sherpa-onnx-sys` version in `src-tauri/Cargo.lock` (1.13.8 at the time of writing). Use an absolute path: the build script does not run in your current directory.

```sh
mkdir -p "$HOME/sherpa-archives"
curl -L -o "$HOME/sherpa-archives/sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2" \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2
# The archive is linked into Voxy: check it against the pinned SHA-256 first.
(cd "$HOME/sherpa-archives" && grep ' sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2$' \
  /path/to/voxy/src-tauri/sherpa-onnx.sha256 | shasum -a 256 -c -)
SHERPA_ONNX_ARCHIVE_DIR="$HOME/sherpa-archives" cargo tauri build
```

After the first successful download, the archive is cached in `src-tauri/target/sherpa-onnx-prebuilt/`. The crate's build script checks no hash; CI and release builds download the archive themselves and check it against [`src-tauri/sherpa-onnx.sha256`](../src-tauri/sherpa-onnx.sha256).

Alternatively, set `SHERPA_ONNX_LIB_DIR` to the `lib` folder inside an extracted archive (for example `…/sherpa-onnx-v1.13.8-osx-arm64-static-lib/lib`, the folder that holds the `.a` files). It also needs an absolute path.

On Windows, see also [If the build fails](../BUILD_WINDOWS.md#if-the-build-fails) in the Windows guide.

## Data locations

| What | macOS | Windows |
|---|---|---|
| Settings, history, discovery cache (`settings.json`, `history.json`, `discovery.json`) | `~/Library/Application Support/com.dizraid.voice/` | `%LOCALAPPDATA%\com.dizraid.voice\` |
| Models | `~/Library/Application Support/com.dizraid.voice/models/` | `%LOCALAPPDATA%\com.dizraid.voice\models\` |
| Log | `~/Library/Logs/Voxy.log` | `%LOCALAPPDATA%\com.dizraid.voice\Voxy.log` |
| Web view data | `~/Library/WebKit/com.dizraid.voice` and the other folders in [Uninstalling on macOS](#uninstalling-on-macos) | `%LOCALAPPDATA%\com.dizraid.voice\EBWebView\` |
| API keys | Keychain, service `com.dizraid.voice` | Credential Manager, one generic credential per provider named `api-key:<provider>.com.dizraid.voice` |
| The app | `/Applications/Voxy.app` | `%LOCALAPPDATA%\Voxy\` |

The folder is named after the bundle identifier, which is older than the name Voxy.

On Windows, everything Voxy writes is in that one local folder. It keeps no files in the roaming profile (`%APPDATA%`), so a roaming domain profile does not carry the models from PC to PC (the API keys are in Credential Manager, which Windows may roam with the profile). Earlier Windows builds kept settings, history and models in `%APPDATA%\com.dizraid.voice\` and the log in `%LOCALAPPDATA%\Voxy.log`; Voxy does not read or move them, and the [uninstall command](#uninstalling-on-windows) deletes them.

## Uninstalling Voxy

### Uninstalling on macOS

The installer can remove Voxy for you:

```sh
curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --uninstall
```

Besides the app, this deletes the downloaded models, settings, history and saved API keys, and resets Voxy's privacy entries (Accessibility, Input Monitoring and Microphone). To keep the models, settings, history and API keys, add `--keep-data`:

```sh
curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --uninstall --keep-data
```

To do it by hand, or to check that nothing is left:

1. Turn off Launch at login in Settings → General, then quit Voxy.
2. Delete the app, the `com.dizraid.voice` folder and the log file listed [above](#data-locations).
3. Delete the data macOS keeps for the app under the same identifier: `~/Library/WebKit/com.dizraid.voice`, `~/Library/Caches/com.dizraid.voice`, `~/Library/HTTPStorages/com.dizraid.voice`, `~/Library/Saved Application State/com.dizraid.voice.savedState` and `~/Library/Preferences/com.dizraid.voice.plist`. If you deleted the app before turning off Launch at login, also delete `~/Library/LaunchAgents/Voxy.plist`.
4. Remove its Keychain items (service `com.dizraid.voice`, one item per online provider) and its privacy entries (Accessibility, Input Monitoring and Microphone). Repeat the `security` command until it reports that the item could not be found:

   ```sh
   security delete-generic-password -s com.dizraid.voice
   tccutil reset Accessibility com.dizraid.voice
   tccutil reset ListenEvent com.dizraid.voice
   tccutil reset Microphone com.dizraid.voice
   ```

### Uninstalling on Windows

The PowerShell installer can remove Voxy for you. Options need this script block form, because `irm … | iex` cannot pass them:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -Uninstall
```

It stops Voxy, runs Voxy's uninstaller and removes the "Start with Windows" entry. Then it deletes the models, settings, history and log (`%LOCALAPPDATA%\com.dizraid.voice`), the folder earlier builds used (`%APPDATA%\com.dizraid.voice`) and the saved API keys in Credential Manager; if any key is left, it prints the `cmdkey /delete:` command for it. To keep the models, settings, history and API keys (and the folder of earlier builds), add `-KeepData`; the web view cache and the log are still deleted:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -Uninstall -KeepData
```

Either way it also deletes `%LOCALAPPDATA%\Voxy.log`, where earlier builds wrote their log.

To do it by hand, or to check that nothing is left:

1. Turn off Start with Windows in Settings → General, then quit Voxy (tray icon → Quit Voxy).
2. Uninstall Voxy in Windows Settings → Apps → Installed apps (on Windows 10, Apps & features).
3. Delete the `%LOCALAPPDATA%\com.dizraid.voice` folder, and `%LOCALAPPDATA%\Voxy` if it is still there. If an earlier build left `%APPDATA%\com.dizraid.voice` or `%LOCALAPPDATA%\Voxy.log`, delete them too.
4. Remove the API keys from Credential Manager (Control Panel → Credential Manager → Windows Credentials, the entries that end in `.com.dizraid.voice`), or in PowerShell. `cmdkey /list` shows what is left:

   ```powershell
   cmdkey /delete:api-key:openai.com.dizraid.voice
   cmdkey /delete:api-key:groq.com.dizraid.voice
   cmdkey /delete:api-key:elevenlabs.com.dizraid.voice
   cmdkey /list | Select-String dizraid
   ```

5. Start with Windows is the `Voxy` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (Task Manager shows it under Startup apps). If you uninstalled Voxy before turning it off, delete the value and the state Task Manager keeps for it:

   ```powershell
   Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Voxy -ErrorAction SilentlyContinue
   Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' -Name Voxy -ErrorAction SilentlyContinue
   ```
