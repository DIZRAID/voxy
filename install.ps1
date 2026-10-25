<#
.SYNOPSIS
Installs, updates or uninstalls Voxy (push-to-talk dictation) on Windows 10/11 x64.

.DESCRIPTION
Install or update (a normal PowerShell window, no administrator rights):
    irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1 | iex

With options (iex cannot pass them, so run the script as a script block):
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -Uninstall
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -Uninstall -KeepData
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/DIZRAID/voxy/main/install.ps1))) -NoLaunch

An install or update finds the latest release through the GitHub API (its tag
must look like v1.2.3), downloads its installer (the asset whose name ends in
_x64-setup.exe) over HTTPS into a new temporary folder, checks its SHA-256
when GitHub lists one, stops the running Voxy, runs the installer silently
(per user, into %LOCALAPPDATA%\Voxy) and starts Voxy. Run it again to update.

An uninstall runs Voxy's own uninstaller silently and removes the "Start with
Windows" entry, then the models, settings, history and logs
(%LOCALAPPDATA%\com.dizraid.voice) and the API keys in Credential Manager.
Both are safe to run again.

All code is inside functions and Main runs on the last line, so a partly
downloaded script does nothing. Run through iex, it never calls exit (that
would close the window) and keeps its settings out of your session.

.PARAMETER Uninstall
Remove Voxy, its models, settings, history, logs and API keys.

.PARAMETER KeepData
With -Uninstall: keep the downloaded models, settings, history and API keys.

.PARAMETER NoLaunch
Install or update without starting Voxy afterwards.

.NOTES
Environment:
  GITHUB_TOKEN    optional; sent to api.github.com only (higher rate limit,
                  private repository). If GitHub rejects it, the installer
                  goes on without it.
  NO_COLOR        set to turn off coloured output.
  VOXY_INSTALLER  for testing this script: install this local -setup.exe
                  instead of downloading one.
Exit codes (when run as a file): 0 done, 1 failed or unsupported system,
2 options that do not go together.
#>
[CmdletBinding()]
param([switch]$Uninstall, [switch]$KeepData, [switch]$NoLaunch)

$VoxyRepo = 'DIZRAID/voxy'
$VoxyBundleId = 'com.dizraid.voice'              # identifier in tauri.conf.json
$VoxyProviders = @('openai', 'groq', 'elevenlabs') # online providers (online.rs)
$VoxyUninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Voxy'
$VoxyFailed = 'voxy-install-failed'              # marks errors already printed

# ------------------------------------------------------------------ output

function Write-Marked([string]$Marker, [string]$Color, [string]$Text) {
    if (-not $env:NO_COLOR -and -not [Console]::IsOutputRedirected) {
        Write-Host $Marker -ForegroundColor $Color -NoNewline
        Write-Host $Text
    } else {
        Write-Host "$Marker$Text"
    }
}
function Write-Step([string]$Text) { Write-Marked '==> ' 'Cyan' $Text }
function Write-Done([string]$Text) { Write-Marked '==> ' 'Green' $Text }
function Write-Warn([string]$Text) { Write-Marked 'Warning: ' 'Yellow' $Text }
function Write-Fail([string]$Text) { Write-Marked 'Error: ' 'Red' $Text }
function Write-Info([string]$Text) { Write-Host "    $Text" }

# Prints the error and stops; Main turns it into exit code 1.
function Stop-Install([string]$Message) {
    Write-Fail $Message
    throw $VoxyFailed
}

# ----------------------------------------------------------------- helpers

function Test-ScriptFile { return [bool]$MyInvocation.MyCommand.ScriptBlock.File }

function Test-Elevated {
    try {
        $principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

function Get-WindowsBuild {
    try {
        $key = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
        return [int]$key.CurrentBuildNumber
    } catch {
        return [Environment]::OSVersion.Version.Build
    }
}

# HTTP status of a failed Invoke-RestMethod/Invoke-WebRequest (5.1 and 7),
# 0 when there was no answer (DNS, TLS, no network).
function Get-HttpStatus($ErrorRecord) {
    $ex = $ErrorRecord.Exception
    if ($ex.PSObject.Properties['Response'] -and $ex.Response) {
        try { return [int]$ex.Response.StatusCode } catch { }
    }
    return 0
}

# A registry path value without quotes or an icon index: the NSIS installer
# writes "C:\...\Voxy.exe" (with the quotes).
function Get-RegPath($Key, [string]$Name) {
    $prop = $Key.PSObject.Properties[$Name]
    if (-not $prop -or -not $prop.Value) { return '' }
    $value = ([string]$prop.Value).Trim()
    if ($value -match '^"([^"]*)"') { return $Matches[1] }
    return ($value -replace ',\s*-?[0-9]+$', '')
}

# What the installer registered for this user, or $null.
function Get-InstalledVoxy {
    $key = Get-ItemProperty -LiteralPath $VoxyUninstallKey -ErrorAction SilentlyContinue
    if (-not $key) { return $null }
    $uninstaller = Get-RegPath $key 'UninstallString'
    $dir = Get-RegPath $key 'InstallLocation'
    if (-not $dir -and $uninstaller) { $dir = Split-Path -Parent $uninstaller }
    $exe = Get-RegPath $key 'DisplayIcon'
    if (-not $exe -and $dir) { $exe = Join-Path $dir 'Voxy.exe' }
    $version = ''
    if ($key.PSObject.Properties['DisplayVersion']) { $version = [string]$key.DisplayVersion }
    return [pscustomobject]@{ Dir = $dir; Exe = $exe; Uninstaller = $uninstaller; Version = $version }
}

# Stops every Voxy of this Windows session (only one can run at a time).
# Returns the path of the first one stopped, to start it again on failure.
function Stop-Voxy {
    $session = (Get-Process -Id $PID).SessionId
    $procs = @(Get-Process -Name 'Voxy' -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session })
    $first = $null
    foreach ($proc in $procs) {
        Write-Info "Stopping Voxy (PID $($proc.Id))"
        if (-not $first) { try { $first = $proc.Path } catch { } }
        try { Stop-Process -Id $proc.Id -Force -ErrorAction Stop } catch { Write-Warn "Could not stop Voxy (PID $($proc.Id)): $($_.Exception.Message)" }
    }
    foreach ($proc in $procs) { try { $null = $proc.WaitForExit(10000) } catch { } }
    return $first
}

# Runs a program from its own folder (not the current directory) and waits
# for that process only (Start-Process -Wait would also wait for anything it
# starts). Returns its exit code.
function Invoke-AndWait([string]$Path, [string]$Arguments, [int]$Minutes) {
    $proc = Start-Process -FilePath $Path -ArgumentList $Arguments -WorkingDirectory (Split-Path -Parent $Path) -PassThru
    $null = $proc.Handle  # keeps the handle, so ExitCode is readable after the exit
    if (-not $proc.WaitForExit($Minutes * 60000)) { Stop-Install "$Path did not finish within $Minutes minutes." }
    return $proc.ExitCode
}

# ------------------------------------------------------------------ install

function Test-InstallSystem {
    if (-not [Environment]::Is64BitOperatingSystem) { Stop-Install 'Voxy needs 64-bit Windows 10 or 11.' }
    if ("$env:PROCESSOR_ARCHITECTURE $env:PROCESSOR_ARCHITEW6432" -match 'ARM64') {
        Write-Warn 'This PC has an ARM processor. Voxy is built for x64 and runs under emulation there, so speech recognition is slower.'
    }
    $build = Get-WindowsBuild
    if ($build -gt 0 -and $build -lt 17763) {
        Write-Warn "Voxy needs Windows 10 version 1809 (build 17763) or later, and this PC has build $build. It may not start."
    }
}

# Finds the latest release and its installer. Returns Tag, Asset, UseToken.
function Get-LatestRelease {
    $useToken = [bool]$env:GITHUB_TOKEN
    Write-Step 'Looking up the latest Voxy release'
    while ($true) {
        $headers = @{ Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2022-11-28' }
        if ($useToken) { $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN" }
        try {
            $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$VoxyRepo/releases/latest" `
                -Headers $headers -UserAgent 'voxy-install.ps1' -TimeoutSec 60
            break
        } catch {
            $err = $_
            $status = Get-HttpStatus $err
            if ($status -eq 401 -and $useToken) {
                # A public repository needs no token; an expired one must not block.
                Write-Warn 'GitHub rejected GITHUB_TOKEN (HTTP 401), so the installer goes on without it.'
                $useToken = $false
            } elseif ($status -eq 404) {
                Stop-Install "No published Voxy release was found at https://github.com/$VoxyRepo/releases (if the repository is private, set GITHUB_TOKEN)."
            } elseif ($status -eq 403 -or $status -eq 429) {
                Stop-Install 'The GitHub API rate limit is reached. Try again in an hour, or set GITHUB_TOKEN.'
            } elseif ($status -eq 0) {
                Stop-Install "Could not reach api.github.com ($($err.Exception.Message)). Check your internet connection and try again."
            } else {
                Stop-Install "The GitHub API answered with HTTP $status. Try again later."
            }
        }
    }
    # Only the release workflow makes releases, always from tags like v1.2.3:
    # anything else was not made by it, so stop.
    $tag = [string]$release.tag_name
    if ($tag -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+\z') {
        $shown = $tag -replace '[^A-Za-z0-9._-]', ''
        if ($shown.Length -gt 40) { $shown = $shown.Substring(0, 40) }
        Stop-Install "The latest release has an unexpected tag ($shown), so the Voxy release workflow did not make it. Not installing it. See https://github.com/$VoxyRepo/releases"
    }
    $asset = @($release.assets | Where-Object { [string]$_.name -like '*_x64-setup.exe' }) | Select-Object -First 1
    if (-not $asset) {
        Stop-Install "The latest release ($tag) has no Windows installer (*_x64-setup.exe). See https://github.com/$VoxyRepo/releases"
    }
    return [pscustomobject]@{ Tag = $tag; Asset = $asset; UseToken = $useToken }
}

# Downloads the installer to $File and checks its SHA-256.
function Save-Installer($Release, [string]$File) {
    $asset = $Release.Asset
    $headers = @{}
    if ($Release.UseToken) {
        # The API URL works for private repositories too; the token is dropped
        # on the redirect to GitHub's download host (5.1 and 7 both do that).
        $url = [string]$asset.url
        $headers['Accept'] = 'application/octet-stream'
        $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN"
        $prefix = 'https://api.github.com/'
    } else {
        $url = [string]$asset.browser_download_url
        $prefix = 'https://github.com/'
    }
    if (-not $url.StartsWith($prefix, [StringComparison]::Ordinal)) { Stop-Install "Unexpected download URL: $url" }
    Write-Step "Downloading $($asset.name) ($($Release.Tag), $([math]::Round($asset.size / 1MB)) MB)"
    for ($try = 1; ; $try++) {
        try {
            Invoke-WebRequest -Uri $url -Headers $headers -UserAgent 'voxy-install.ps1' -OutFile $File -UseBasicParsing
            break
        } catch {
            if ($try -ge 3) { Stop-Install "The download failed: $url ($($_.Exception.Message))" }
            Start-Sleep -Seconds 2
        }
    }
    $digest = ''
    if ($asset.PSObject.Properties['digest'] -and $asset.digest) { $digest = [string]$asset.digest }
    if ($digest -like 'sha256:*') {
        $actual = (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash
        if ($actual -ne $digest.Substring(7)) { Stop-Install 'The downloaded file is damaged (SHA-256 mismatch). Run the installer again.' }
        Write-Info 'SHA-256 checked'
    } else {
        Write-Warn 'GitHub lists no SHA-256 for this file, so the download could not be checked.'
    }
}

function Assert-WindowsProgram([string]$File) {
    $bytes = New-Object byte[] 2
    $stream = [IO.File]::OpenRead($File)
    try { $count = $stream.Read($bytes, 0, 2) } finally { $stream.Dispose() }
    if ($count -ne 2 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        Stop-Install "$File is not a Windows program (damaged or incomplete download?). Not running it."
    }
}

function Install-Voxy {
    Test-InstallSystem
    $elevated = Test-Elevated
    if ($elevated) {
        Write-Warn 'This PowerShell runs as administrator. Voxy installs for the account of this window and should not run elevated, so it is not started from here: start it from the Start menu.'
    }
    $before = Get-InstalledVoxy
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('voxy-install-' + [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $tmp
    $restart = $null
    try {
        $setup = Join-Path $tmp 'Voxy-setup.exe'
        if ($env:VOXY_INSTALLER) {
            if (-not (Test-Path -LiteralPath $env:VOXY_INSTALLER -PathType Leaf)) { Stop-Install "VOXY_INSTALLER: no such file: $env:VOXY_INSTALLER" }
            $source = (Resolve-Path -LiteralPath $env:VOXY_INSTALLER).ProviderPath
            Write-Step "Using the local installer $source (VOXY_INSTALLER)"
            Copy-Item -LiteralPath $source -Destination $setup
            $version = '(local installer)'
        } else {
            $release = Get-LatestRelease
            Save-Installer $release $setup
            $version = $release.Tag.Substring(1)
        }
        Assert-WindowsProgram $setup

        if (-not $before) {
            Write-Step "Installing Voxy $version"
        } elseif ($before.Version -eq $version) {
            Write-Step "Reinstalling Voxy $version"
        } else {
            Write-Step "Updating Voxy ($($before.Version) -> $version)"
        }
        $restart = Stop-Voxy
        # /S: silent. The installer also stops a running Voxy, installs per user
        # (no administrator rights) and installs WebView2 when it is missing.
        $code = Invoke-AndWait $setup '/S' 10
        if ($code -ne 0) { Stop-Install "The Voxy installer failed (exit code $code)." }
        $restart = $null

        $installed = Get-InstalledVoxy
        $exe = ''
        if ($installed -and $installed.Exe -and (Test-Path -LiteralPath $installed.Exe -PathType Leaf)) {
            $exe = $installed.Exe
        } elseif (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'Voxy\Voxy.exe') -PathType Leaf) {
            $exe = Join-Path $env:LOCALAPPDATA 'Voxy\Voxy.exe'
        } else {
            Stop-Install "The installer finished, but Voxy.exe is not where it should be ($env:LOCALAPPDATA\Voxy)."
        }
        if (-not $NoLaunch -and -not $elevated) {
            try { Start-Process -FilePath $exe -WorkingDirectory (Split-Path -Parent $exe) }
            catch { Write-Warn "Could not start Voxy: $($_.Exception.Message). Start it from the Start menu." }
        }
    } finally {
        if ($restart -and (Test-Path -LiteralPath $restart -PathType Leaf)) {
            try {
                Start-Process -FilePath $restart -WorkingDirectory (Split-Path -Parent $restart)
                Write-Warn "Started the Voxy that was running before ($restart)."
            } catch { }
        }
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($installed -and $installed.Version) { $version = $installed.Version }
    $data = Join-Path $env:LOCALAPPDATA $VoxyBundleId
    Write-Host ''
    if (-not $before) {
        Write-Done "Voxy $version is installed in $(Split-Path -Parent $exe)."
    } elseif ($before.Version -eq $version) {
        Write-Done "Voxy $version is reinstalled."
    } else {
        Write-Done "Voxy is updated to $version."
    }
    Write-Info '- Voxy lives in the notification area (the tray icon; it may be hidden'
    Write-Info '  under the ^ arrow). Click it to open Settings. It is in the Start menu too.'
    if (-not $before) {
        Write-Info '- Settings opens on the first launch while the default speech model'
        Write-Info '  (670 MB) downloads; progress is in Settings > Model.'
    }
    Write-Info '- Dictate: hold Right Ctrl, speak, release. The text is pasted at the cursor.'
    Write-Info "- Models, settings and history: $data"
    Write-Info "  Log: $data\Voxy.log"
    Write-Info '- To update, run the same command again.'
}

# ---------------------------------------------------------------- uninstall

function Remove-PathIfPresent($Removed, [string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    # Voxy's WebView2 processes can hold files for a moment after it stops.
    for ($try = 1; ; $try++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            $Removed.Add($Path)
            return
        } catch {
            if ($try -ge 3) { Write-Warn "Could not delete ${Path}: $($_.Exception.Message)"; return }
            Start-Sleep -Seconds 1
        }
    }
}

# "Start with Windows" (tauri-plugin-autostart): HKCU Run value "Voxy", plus
# the enabled/disabled state Task Manager keeps under StartupApproved.
function Remove-Autostart($Removed) {
    $run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    $entry = Get-ItemProperty -LiteralPath $run -Name 'Voxy' -ErrorAction SilentlyContinue
    if ($entry) {
        if ([string]$entry.Voxy -notmatch 'voxy\.exe') {
            Write-Warn "Left $run\Voxy alone: it does not start Voxy.exe."
            return
        }
        Remove-ItemProperty -LiteralPath $run -Name 'Voxy'
        $Removed.Add("Start with Windows ($run\Voxy)")
    }
    if (Get-ItemProperty -LiteralPath $approved -Name 'Voxy' -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -LiteralPath $approved -Name 'Voxy'
    }
}

# Credential Manager targets of Voxy's API keys that cmdkey lists, as cmdkey
# prints them (e.g. LegacyGeneric:target=api-key:openai.com.dizraid.voice).
function Get-VoxyCredentials([string]$Cmdkey) {
    $ErrorActionPreference = 'Continue'  # native stderr is not a script error
    $pattern = ':\s+(\S+' + [regex]::Escape(".$VoxyBundleId") + ')\s*$'
    foreach ($line in @(& $Cmdkey '/list' 2>$null)) {
        if ("$line" -match $pattern) { $Matches[1] }
    }
}

# keyring stores each key as the generic credential api-key:<provider>.<id>.
function Remove-Credentials($Removed) {
    $cmdkey = Join-Path $env:SystemRoot 'System32\cmdkey.exe'
    if (-not (Test-Path -LiteralPath $cmdkey -PathType Leaf)) {
        Write-Warn 'cmdkey.exe was not found, so API keys stay in Credential Manager.'
        return
    }
    $ErrorActionPreference = 'Continue'
    $targets = @($VoxyProviders | ForEach-Object { 'api-key:{0}.{1}' -f $_, $VoxyBundleId }) + @(Get-VoxyCredentials $cmdkey)
    foreach ($target in @($targets | Select-Object -Unique)) {
        $null = & $cmdkey "/delete:$target" 2>&1
        if ($LASTEXITCODE -eq 0) { $Removed.Add("Credential Manager: $target") }
    }
    $left = @(Get-VoxyCredentials $cmdkey)
    if ($left.Count -gt 0) {
        Write-Warn 'Some API keys are still in Credential Manager. Delete them with:'
        foreach ($target in $left) { Write-Info "cmdkey /delete:$target" }
    }
}

function Uninstall-Voxy {
    Write-Step 'Uninstalling Voxy'
    $removed = New-Object System.Collections.Generic.List[string]
    $null = Stop-Voxy
    $installed = Get-InstalledVoxy
    if (-not $installed) {
        Write-Info 'Voxy is not installed for this user (no uninstaller is registered).'
    } elseif (-not $installed.Uninstaller -or -not (Test-Path -LiteralPath $installed.Uninstaller -PathType Leaf) -or
        (Split-Path -Leaf $installed.Uninstaller) -ne 'uninstall.exe') {
        Write-Warn "Voxy is registered, but its uninstaller is missing ($($installed.Uninstaller)). Delete $($installed.Dir) yourself."
    } else {
        # _?= runs the NSIS uninstaller in place instead of a copy in %TEMP%
        # (whose start process exits at once), so waiting for it waits for the
        # uninstall. NSIS wants it last and unquoted, even with spaces. It
        # cannot delete itself then, so its file and the empty folder go below.
        $dir = Split-Path -Parent $installed.Uninstaller
        Write-Info "Running $($installed.Uninstaller)"
        $code = Invoke-AndWait $installed.Uninstaller "/S _?=$dir" 5
        if ($code -ne 0) { Stop-Install "The Voxy uninstaller failed (exit code $code). Uninstall Voxy in Settings > Apps instead." }
        Remove-Item -LiteralPath $installed.Uninstaller -Force -ErrorAction SilentlyContinue
        if (-not @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue).Count) {
            Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue
        }
        $removed.Add("Voxy $($installed.Version) ($dir)")
    }

    Remove-Autostart $removed
    $data = Join-Path $env:LOCALAPPDATA $VoxyBundleId
    if ($KeepData) {
        Write-Info "Keeping models, settings, history and API keys ($data, Credential Manager)."
        Remove-PathIfPresent $removed (Join-Path $data 'EBWebView')  # WebView2 cache
        Remove-PathIfPresent $removed (Join-Path $data 'Voxy.log')
    } else {
        Remove-PathIfPresent $removed $data
        Remove-PathIfPresent $removed (Join-Path $env:APPDATA $VoxyBundleId)  # roaming folder of early builds
        Remove-Credentials $removed
    }
    Remove-PathIfPresent $removed (Join-Path $env:LOCALAPPDATA 'Voxy.log')  # log path of early builds

    Write-Host ''
    if ($removed.Count -gt 0) {
        Write-Done 'Voxy is uninstalled. Removed:'
        foreach ($item in $removed) { Write-Info $item }
    } else {
        Write-Done 'Voxy was not installed; nothing to remove.'
    }
}

# --------------------------------------------------------------------- main

# Returns the exit code. Preferences set here stay in this function's scope,
# so a run through iex leaves the session as it was.
function Main {
    Set-StrictMode -Version 2
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # the 5.1 progress bar slows downloads down a lot
    try {
        if ($KeepData -and -not $Uninstall) { Write-Fail '-KeepData only works with -Uninstall.'; return 2 }
        if ($NoLaunch -and $Uninstall) { Write-Fail '-NoLaunch only works when installing.'; return 2 }
        if ($env:OS -ne 'Windows_NT') {
            Stop-Install "This installer is for Windows. On a Mac, run in Terminal: curl -fsSL https://raw.githubusercontent.com/$VoxyRepo/main/install.sh | bash"
        }
        # GitHub accepts TLS 1.2 or later; Windows PowerShell 5.1 may offer only 1.0/1.1.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        if ($Uninstall) { $null = Uninstall-Voxy } else { $null = Install-Voxy }
        return 0
    } catch {
        if ("$_" -ne $VoxyFailed) {
            Write-Fail "$($_.Exception.Message) (install.ps1 line $($_.InvocationInfo.ScriptLineNumber)). Please report it at https://github.com/$VoxyRepo/issues"
        }
        return 1
    }
}

# As a file (powershell -File, & .\install.ps1): exit with Main's code. Through
# iex or a script block, exit would close the PowerShell window, so the code
# only goes to $LASTEXITCODE.
if (Test-ScriptFile) { exit (Main) } else { $global:LASTEXITCODE = Main }
