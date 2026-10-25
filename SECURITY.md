# Security policy

## Supported versions

Only the latest release is supported: the one the install command installs
(`https://github.com/DIZRAID/voxy/releases/latest`). Fixes ship as a new
release; update with the same install command.

## Reporting a vulnerability

Please report security issues privately, not in a public issue:

- GitHub: open the repository's **Security** tab and click **Report a
  vulnerability** (<https://github.com/DIZRAID/voxy/security/advisories/new>).

Include the Voxy and macOS (or Windows) version, the steps to reproduce, and
what an attacker could do with it. English or Russian is fine.

You can expect an acknowledgement within 7 days. The goal is a fix or a
published advisory within 30 days, depending on severity; you will be kept
informed and credited in the advisory if you wish.

## Scope

In scope:

- the app: `src-tauri/` (Rust) and `ui/` (the Settings and island pages,
  their IPC permissions and Content Security Policy);
- `install.sh`, `install.ps1` and the one-command install, update and
  uninstall paths on macOS and Windows;
- the release workflow and the published release assets;
- how models, the sherpa-onnx native libraries and model discovery data are
  downloaded and verified;
- handling of API keys (Keychain, Windows Credential Manager), dictation
  history, the clipboard and the global hotkey.

Out of scope:

- the online providers themselves (OpenAI, Groq, ElevenLabs) and the contents
  of third-party speech models;
- attacks that need code already running as your user account, or physical
  access to an unlocked computer;
- the lack of an Apple Developer ID signature and notarization, and of a
  Windows code signature (the SmartScreen warning), which are known
  limitations (see the README).

## Verifying a download

Release files are built by `.github/workflows/release.yml` and carry a signed
build provenance attestation. With the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify Voxy_<version>_aarch64.app.tar.gz -R DIZRAID/voxy \
  --signer-workflow DIZRAID/voxy/.github/workflows/release.yml
```

The same command works for the `.dmg` and for the Windows installer,
`Voxy_<version>_x64-setup.exe` (in PowerShell, put it on one line or end the
first line with a backtick instead of `\`).

`install.sh` checks the SHA-256 that GitHub lists for the downloaded archive,
the app's bundle identifier, code signature and entitlements, and that the
release tag has the `v<major>.<minor>.<patch>` form. Releases are signed ad hoc
(not with an Apple Developer ID), so the code signature shows that the bundle
is intact, not who built it; the attestation is what ties a file to this
repository's release workflow.

`install.ps1` checks the SHA-256 that GitHub lists for the downloaded
installer, that the file is a Windows program, that it comes from
`github.com` (or `api.github.com` when `GITHUB_TOKEN` is set) over HTTPS, and
that the release tag has the `v<major>.<minor>.<patch>` form. The installer is
not code-signed, so Windows cannot tell who built it; here too, the
attestation is what ties it to the release workflow.
