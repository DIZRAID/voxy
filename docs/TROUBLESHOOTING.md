# Troubleshooting

[Back to the README](../README.md)

- [Logs](#logs)
- [Messages on the island](#messages-on-the-island)
- [The hotkey does nothing](#the-hotkey-does-nothing)
- [Text is recognized but not pasted](#text-is-recognized-but-not-pasted)
- [Every recording ends with "Didn't catch that"](#every-recording-ends-with-didnt-catch-that)
- [The menu bar icon is missing](#the-menu-bar-icon-is-missing)
- [A model download fails](#a-model-download-fails)
- [The build fails while downloading sherpa-onnx](#the-build-fails-while-downloading-sherpa-onnx)
- [Data locations](#data-locations)
- [Uninstalling Voxy](#uninstalling-voxy)

## Logs

Logs are written to `~/Library/Logs/Voxy.log` (on Windows, `%LOCALAPPDATA%\Voxy.log`). Set `VOXY_LOG_STDERR=1` to send them to stderr instead. `RUST_LOG` overrides the default `info` level. Log messages, model download errors shown in Settings → Model, and the `model_smoke` output are currently in Russian.

## Messages on the island

| Message | Meaning |
|---|---|
| Model is not ready yet | Shown when you press the hotkey: the active model is not downloaded or failed to load. Check Settings → Model |
| Model is not ready | Shown after a recording: the local model failed to load, so the recording was not transcribed. Check Settings → Model and the [log](#logs) |
| Online engine unavailable | The selected online provider has no saved API key, or macOS denied Voxy access to the key in the Keychain (this can happen after an update or a rebuild). Add the key again in Settings → Model, or choose Always Allow when macOS asks |
| Microphone unavailable | The input device could not be opened |
| Didn't catch that | No speech was recognized |
| Paste failed — text kept in clipboard | The synthetic ⌘V could not be sent, for example because Accessibility was turned off while Voxy was running. Paste with ⌘V yourself; the text is also in History |
| `<Provider>: invalid API key` / `rate limit or no credits` / `No connection to <Provider>` | An error from the online provider |
| `<Provider>: server error <code>` | The provider returned a server error. Try again later; the details are in the [log](#logs) |
| Something went wrong | An internal error; the recording was discarded. See the [log](#logs) |

## The hotkey does nothing

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

- After a reinstall, make sure the old copy is not still running.
- If you started Voxy with `cargo tauri dev`, grant the permissions to your terminal app.
- Make sure you are pressing the key shown in Settings → General → Hotkey.
- Check the log. `accessibility=true input_monitoring=true`, followed by a line that contains `event tap`, means the hotkey is active:

  ```sh
  grep -E 'accessibility=|event tap' ~/Library/Logs/Voxy.log | tail
  ```

## Text is recognized but not pasted

If the island says "Paste failed — text kept in clipboard", the text is in the clipboard: paste it with ⌘V, and check that Accessibility is still granted.

Otherwise the ⌘V was sent but the app did not accept it (for example, no text field had focus). The clipboard has already been restored to what it held before, so copy the text from Settings → History.

## Every recording ends with "Didn't catch that"

Check System Settings → Privacy & Security → Microphone: Voxy must be turned on there (with `cargo tauri dev`, your terminal app). Also check the input device in Settings → General → Microphone.

## The menu bar icon is missing

On a MacBook with a notch, a full menu bar can hide Voxy's icon. Open Voxy again from `/Applications`: since only one copy runs at a time, this brings up the Settings window of the running copy.

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
SHERPA_ONNX_ARCHIVE_DIR="$HOME/sherpa-archives" cargo tauri build
```

After the first successful download, the archive is cached in `src-tauri/target/sherpa-onnx-prebuilt/`.

Alternatively, set `SHERPA_ONNX_LIB_DIR` to the `lib` folder inside an extracted archive (for example `…/sherpa-onnx-v1.13.8-osx-arm64-static-lib/lib`, the folder that holds the `.a` files). It also needs an absolute path.

On Windows, see also [If the build fails](../BUILD_WINDOWS.md#if-the-build-fails) in the Windows guide.

## Data locations

| What | macOS | Windows |
|---|---|---|
| Settings, history, discovery cache (`settings.json`, `history.json`, `discovery.json`) | `~/Library/Application Support/com.dizraid.voice/` | `%APPDATA%\com.dizraid.voice\` |
| Models | `~/Library/Application Support/com.dizraid.voice/models/` | `%APPDATA%\com.dizraid.voice\models\` |
| Log | `~/Library/Logs/Voxy.log` | `%LOCALAPPDATA%\Voxy.log` |
| API keys | Keychain, service `com.dizraid.voice` | Credential Manager |

The folder is named after the bundle identifier, which is older than the name Voxy.

## Uninstalling Voxy

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
