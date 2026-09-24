#!/bin/bash
# Voxy installer for macOS (Apple Silicon, macOS 14 or later).
#
# Install or update:
#   curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash
# If the hotkey does nothing after an update (resets Accessibility and Input
# Monitoring for Voxy, then restarts it; downloads nothing):
#   curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --reset-permissions
# Uninstall (the app, models, settings, logs, API keys, privacy entries):
#   curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --uninstall
# Uninstall, but keep downloaded models, settings and API keys:
#   curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --uninstall --keep-data
#
# An install or update:
#   1. finds the latest release through the GitHub API (its tag must look like
#      v1.2.3) and downloads its Apple
#      Silicon app (the asset whose name ends in aarch64.app.tar.gz), checks its
#      SHA-256 when GitHub lists one, and checks that it holds Voxy.app with
#      bundle identifier com.dizraid.voice, a valid code signature and, under
#      the hardened runtime, the microphone entitlement;
#   2. copies it next to the installed app under a hidden name, stops every
#      running copy of Voxy (only one copy can run at a time, and a second
#      launch just shows the Settings of the copy that is running), then swaps
#      the two with renames, so Voxy.app is never half-copied;
#   3. installs to /Applications/Voxy.app (~/Applications/Voxy.app when
#      /Applications is not writable); an app of another developer that is
#      also called Voxy.app is never replaced;
#   4. resets Voxy's Accessibility and Input Monitoring entries when they may
#      belong to a different code signature (see below);
#   5. opens Voxy and lists the other copies of Voxy it found.
#
# Why the reset: releases are signed ad hoc, so every version has a different
# designated requirement (codesign -d -r-). macOS keeps one privacy entry per
# bundle identifier and ties it to the signature of the copy it was granted
# to: when the signature changes, the switch still looks on, but the
# permission is denied. The installer resets the two entries when the new
# signature differs from the installed copy's, or when another copy of Voxy
# (found with Spotlight, or running) may hold them: one with a different
# signature, or one outside the Applications folders, which the installer does
# not open (reading files in Desktop, Documents, Downloads or on another volume
# can make macOS ask whether the terminal may access that folder). If the
# hotkey still does nothing, --reset-permissions resets the entries anyway.
#
# No sudo. Files downloaded with curl get no quarantine attribute, so Gatekeeper
# does not block the ad-hoc signed app. Apart from the bash check below, all
# code is inside functions and main runs on the last line, so a partly
# downloaded script does nothing.
#
# Environment:
#   GITHUB_TOKEN   optional; sent to the GitHub API only (higher rate limit,
#                  access to a private repository). If GitHub rejects it, the
#                  installer goes on without it. While the repository is
#                  private, fetching this script needs the token as well:
#                    curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
#                      https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash
#   NO_COLOR       set to turn off coloured output
#
# Exit codes: 0 done, 1 failed or unsupported Mac, 2 bad arguments.
#
# Test knobs, for testing this script locally only (normal installs never set
# them):
#   VOXY_ARCHIVE=<file>       use this local .app.tar.gz instead of downloading;
#                             a bad code signature is then only a warning
#   VOXY_APP_DIR=<dir>        install into / uninstall from <dir> instead of
#                             /Applications (and ~/Applications); without
#                             VOXY_DATA_ROOT, the login item is left alone
#   VOXY_NO_TCC=1             print the tccutil resets instead of running them
#                             (also skips registering the app with LaunchServices)
#   VOXY_NO_LAUNCH=1          do not open the app
#   VOXY_NO_KILL=1            list running copies of Voxy instead of stopping them
#   VOXY_DATA_ROOT=<dir>      fake home folder (LaunchAgents, Library data, Trash);
#                             also skips the Keychain and launchctl, which are
#                             not under $HOME
#   VOXY_FAKE_OS=<name>       pretend `uname -s` printed <name>
#   VOXY_FAKE_ARCH=<arch>     pretend `uname -m` printed <arch> (e.g. x86_64)
#   VOXY_FAKE_MACOS=<version> pretend `sw_vers -productVersion` printed <version>
# Example (changes nothing outside the test folder and a temporary folder; it
# only reads the process list and Spotlight):
#   VOXY_ARCHIVE=$PWD/Voxy_aarch64.app.tar.gz VOXY_APP_DIR=$PWD/apps \
#   VOXY_DATA_ROOT=$PWD/home VOXY_NO_TCC=1 VOXY_NO_LAUNCH=1 VOXY_NO_KILL=1 \
#   bash install.sh

# zsh or another shell (`curl ... | zsh`) would run this script with different
# word splitting; /bin/sh is bash on macOS and works.
if [ -z "${BASH_VERSION:-}" ]; then
  echo "Run the Voxy installer with bash: curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash" >&2
  exit 1
fi

set -euo pipefail

REPO="DIZRAID/voxy"
BUNDLE_ID="com.dizraid.voice"
APP_NAME="Voxy.app"
INSTALL_CMD="curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash"
# The release workflow packs Voxy.app (at the archive root) and uploads it as
# Voxy_<version>_aarch64.app.tar.gz. The API lookup matches on this suffix only.
ASSET_SUFFIX="aarch64.app.tar.gz"
MIN_MACOS="14.0"
PLISTBUDDY="/usr/libexec/PlistBuddy"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
TAB=$(printf '\t')

# ------------------------------------------------------------------ output

setup_colors() {
  if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != "dumb" ]; then
    BOLD=$'\033[1m'
    DIM=$'\033[2m'
    RED=$'\033[31m'
    GREEN=$'\033[32m'
    YELLOW=$'\033[33m'
    BLUE=$'\033[34m'
    RESET=$'\033[0m'
  else
    BOLD="" DIM="" RED="" GREEN="" YELLOW="" BLUE="" RESET=""
  fi
}

step() { printf '%s==>%s %s%s%s\n' "$BLUE" "$RESET" "$BOLD" "$*" "$RESET"; }
info() { printf '    %s\n' "$*"; }
ok() { printf '%s==>%s %s%s%s\n' "$GREEN" "$RESET" "$BOLD" "$*" "$RESET"; }
warn() { printf '%sWarning:%s %s\n' "$YELLOW" "$RESET" "$*" >&2; }
testlog() { printf '    %s[test] %s%s\n' "$DIM" "$*" "$RESET"; }

# Prints an error and exits. Never call it inside $(...): it would only end
# the subshell.
die() {
  printf '%sError:%s %s\n' "$RED" "$RESET" "$1" >&2
  exit "${2:-1}"
}

usage() {
  cat <<'EOF'
Voxy installer for macOS (Apple Silicon, macOS 14 or later)

Install or update:
  curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash

If the hotkey does nothing after an update:
  curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --reset-permissions

Uninstall:
  curl -fsSL https://raw.githubusercontent.com/DIZRAID/voxy/main/install.sh | bash -s -- --uninstall

Options:
  --reset-permissions  reset Voxy's Accessibility and Input Monitoring entries
                       and restart it (downloads nothing)
  --uninstall          remove Voxy, its models, settings, logs, API keys and
                       privacy entries
  --keep-data          with --uninstall: keep downloaded models, settings and
                       API keys
  -h, --help           show this help
EOF
}

# ----------------------------------------------------------------- helpers

is_set() { [ "${1:-0}" != "0" ] && [ -n "${1:-}" ]; }

# Prints a key from an Info.plist; fails if the file or key is missing.
plist_get() {
  "$PLISTBUDDY" -c "Print :$2" "$1" 2>/dev/null
}

# Prints the bundle identifier of the app at $1 and succeeds when it belongs
# to another app. An app without a readable identifier counts as Voxy.
foreign_id() {
  local id
  id=$(plist_get "$1/Contents/Info.plist" CFBundleIdentifier) || id=""
  if [ -n "$id" ] && [ "$id" != "$BUNDLE_ID" ]; then
    printf '%s\n' "$id"
    return 0
  fi
  return 1
}

# version_ge A B: true when version A >= version B (major.minor.patch, numeric).
version_ge() {
  awk -v a="$1" -v b="$2" 'BEGIN {
    split(a, x, "."); split(b, y, ".")
    for (i = 1; i <= 3; i++) {
      p = x[i] + 0; q = y[i] + 0
      if (p > q) exit 0
      if (p < q) exit 1
    }
    exit 0
  }'
}

# Prints the designated requirement of a signed bundle (without the path line
# codesign adds), or fails when the bundle is unsigned or unreadable.
designated_requirement() {
  local out dr
  out=$(codesign -d -r- "$1" 2>&1) || return 1
  dr=$(printf '%s\n' "$out" | sed -n 's/^#* *designated => //p' | sed -n 1p)
  [ -n "$dr" ] || return 1
  printf '%s\n' "$dr"
}

# Prints the string values of "key": "value" pairs in a JSON file, in document
# order. Enough for the GitHub API (URLs and tag names contain no quotes);
# python3 and jq are not guaranteed on a fresh Mac.
json_values() {
  grep -o "\"$1\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$2" |
    sed "s/^\"$1\"[[:space:]]*:[[:space:]]*\"//; s/\"\$//" || true
}

# Prints line number N of a newline-separated list.
nth_line() {
  printf '%s\n' "$1" | sed -n "${2}p"
}

count_lines() {
  if [ -z "$1" ]; then echo 0; else printf '%s\n' "$1" | wc -l | tr -d ' '; fi
}

sha256_of() {
  local sum
  sum=$(shasum -a 256 "$1" 2>/dev/null) || sum=$(openssl dgst -sha256 -r "$1" 2>/dev/null) || return 1
  printf '%s\n' "${sum%% *}"
}

launch_app() {
  if is_set "${VOXY_NO_LAUNCH:-}"; then
    testlog "would run: open -a $1"
  else
    open -a "$1" 2>/dev/null
  fi
}

# ------------------------------------------------------------------ checks

check_system() {
  local mode=$1 os arch version
  os=${VOXY_FAKE_OS:-$(uname -s)}
  if [ "$os" != "Darwin" ]; then
    die "This installer is for macOS, and this system is $os. On Windows, get the -setup.exe from https://github.com/$REPO/releases/latest"
  fi
  [ "$mode" = "install" ] || return 0

  if [ -n "${VOXY_FAKE_ARCH:-}" ]; then
    arch=$VOXY_FAKE_ARCH
  else
    arch=$(uname -m)
    # A shell running under Rosetta reports x86_64 on Apple Silicon too.
    if [ "$arch" != "arm64" ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = "1" ]; then
      arch=arm64
    fi
  fi
  if [ "$arch" != "arm64" ]; then
    die "Voxy runs on Macs with Apple Silicon (M1 or later). This Mac has an Intel processor ($arch), which is not supported yet."
  fi

  version=${VOXY_FAKE_MACOS:-$(sw_vers -productVersion)}
  if ! version_ge "$version" "$MIN_MACOS"; then
    die "Voxy needs macOS 14 Sonoma or later, and this Mac has macOS $version. Update macOS in System Settings > General > Software Update, then run the installer again."
  fi
  MACOS_VERSION=$version

  local tool
  for tool in curl tar codesign ditto xattr mktemp; do
    command -v "$tool" >/dev/null 2>&1 || die "The command '$tool' is missing, and the installer needs it."
  done
  [ -x "$PLISTBUDDY" ] || die "$PLISTBUDDY is missing, and the installer needs it."
}

# ------------------------------------------------ lock, temp dir and cleanup

LOCK_PATH=""
TMP_DIR=""
DEST=""
STAGING_PATH=""
BACKUP_PATH=""
RESTART_PATH=""

# Two installers at once (the command pasted into two windows) could nest one
# copy of Voxy inside the other. The lock is a symlink whose target is the PID
# of the installer that holds it; ln -s creates it atomically.
take_lock() {
  local lock pid
  lock=${TMPDIR:-/tmp}
  lock="${lock%/}/voxy-install.lock"
  if ln -s "$$" "$lock" 2>/dev/null; then
    LOCK_PATH=$lock
    return 0
  fi
  [ -L "$lock" ] || die "Could not create $lock"
  pid=$(readlink "$lock" 2>/dev/null) || pid=""
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    die "Another Voxy installer is running (PID $pid). Let it finish, then run this command again."
  fi
  # Left behind by an installer that was killed.
  rm -f "$lock"
  ln -s "$$" "$lock" 2>/dev/null ||
    die "Another Voxy installer is running. Let it finish, then run this command again."
  LOCK_PATH=$lock
}

cleanup() {
  local rc=$?
  # A second Ctrl-C or a closed window must not interrupt the restore below.
  trap '' INT TERM HUP
  # An interrupted swap: put the previous app back.
  if [ -n "$BACKUP_PATH" ] && [ -e "$BACKUP_PATH" ]; then
    if [ -n "$DEST" ] && [ ! -e "$DEST" ] && [ ! -L "$DEST" ]; then
      mv "$BACKUP_PATH" "$DEST" 2>/dev/null && warn "Put the previous $APP_NAME back."
    else
      rm -rf "$BACKUP_PATH" 2>/dev/null || true
    fi
  fi
  if [ -n "$STAGING_PATH" ]; then rm -rf "$STAGING_PATH" 2>/dev/null || true; fi
  if [ -n "$TMP_DIR" ]; then rm -rf "$TMP_DIR" 2>/dev/null || true; fi
  # The install failed after Voxy was stopped: start the copy that was running.
  if [ "$rc" != "0" ] && [ -n "$RESTART_PATH" ] && [ -d "$RESTART_PATH" ]; then
    launch_app "$RESTART_PATH" && warn "Started the Voxy that was running before ($RESTART_PATH)."
  fi
  if [ -n "$LOCK_PATH" ]; then rm -f "$LOCK_PATH" 2>/dev/null || true; fi
}

make_temp_dir() {
  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/voxy-install.XXXXXX") || die "Could not create a temporary folder."
}

# -------------------------------------------------------- running copies

# Prints "PID<TAB>bundle path" for every copy of Voxy running as this user:
# processes whose executable is <something>.app/Contents/MacOS/voxy (or voice,
# the name older builds used) inside a bundle with our identifier.
list_running() {
  local uid pid puid comm bundle id
  uid=$(id -u)
  ps -axo pid=,uid=,comm= 2>/dev/null | while read -r pid puid comm; do
    [ "$puid" = "$uid" ] || continue
    case "$comm" in
      *.app/Contents/MacOS/voxy | *.app/Contents/MacOS/voice) ;;
      *) continue ;;
    esac
    bundle=${comm%/Contents/MacOS/*}
    id=$(plist_get "$bundle/Contents/Info.plist" CFBundleIdentifier) || id=""
    if [ "$id" = "$BUNDLE_ID" ] || { [ -z "$id" ] && [ "${bundle##*/}" = "$APP_NAME" ]; }; then
      printf '%s\t%s\n' "$pid" "$bundle"
    fi
  done || true
}

# True when the bundle at $1 is in $RUNNING.
is_running_bundle() {
  local pid bundle
  while IFS=$TAB read -r pid bundle; do
    if [ -n "$pid" ] && [ "$bundle" = "$1" ]; then return 0; fi
  done <<EOF
$RUNNING
EOF
  return 1
}

alive_pids() {
  local p alive=""
  for p in "$@"; do
    if kill -0 "$p" 2>/dev/null; then alive="$alive $p"; fi
  done
  printf '%s' "${alive# }"
}

# Stops the copies listed in $RUNNING: SIGTERM, up to 5 seconds, then SIGKILL.
# Sets STOPPED_FIRST to the bundle of the first copy it stopped.
stop_running() {
  STOPPED_FIRST=""
  [ -n "$RUNNING" ] || return 0
  local pid bundle pids="" alive tries=0
  while IFS=$TAB read -r pid bundle; do
    [ -n "$pid" ] || continue
    if is_set "${VOXY_NO_KILL:-}"; then
      testlog "would stop Voxy (PID $pid) running from $bundle"
      continue
    fi
    info "Stopping Voxy (PID $pid) running from $bundle"
    kill -TERM "$pid" 2>/dev/null || true
    pids="$pids $pid"
    if [ -z "$STOPPED_FIRST" ]; then STOPPED_FIRST=$bundle; fi
  done <<EOF
$RUNNING
EOF
  [ -n "$pids" ] || return 0
  # shellcheck disable=SC2086 # word splitting of the PID list is intended
  while [ "$tries" -lt 50 ]; do
    alive=$(alive_pids $pids)
    [ -n "$alive" ] || return 0
    sleep 0.1
    tries=$((tries + 1))
  done
  # shellcheck disable=SC2086
  kill -KILL $alive 2>/dev/null || true
  sleep 0.5
  # shellcheck disable=SC2086
  alive=$(alive_pids $alive)
  if [ -n "$alive" ]; then
    die "Could not stop Voxy (PID $alive). Quit it from the menu bar icon and run the installer again."
  fi
}

# -------------------------------------------------------------- privacy

# Registers the installed app with LaunchServices, so tccutil can resolve the
# bundle identifier on a first install.
register_app() {
  if [ -x "$LSREGISTER" ] && [ -d "$1" ]; then
    "$LSREGISTER" -f "$1" >/dev/null 2>&1 || true
  fi
}

# reset_tcc SERVICE... : resets Voxy's privacy entries for the given services.
# Sets TCC_FAILED to the services that could not be reset.
reset_tcc() {
  local svc
  TCC_FAILED=""
  for svc in "$@"; do
    if is_set "${VOXY_NO_TCC:-}"; then
      testlog "would run: tccutil reset $svc $BUNDLE_ID"
    elif ! tccutil reset "$svc" "$BUNDLE_ID" >/dev/null 2>&1; then
      TCC_FAILED="$TCC_FAILED $svc"
    fi
  done
  [ -z "$TCC_FAILED" ]
}

# True for copies whose signature the installer may read: running ones and
# those in an Applications folder. Reading files in Desktop, Documents,
# Downloads, iCloud Drive or on another volume can make macOS ask whether the
# terminal may access that folder, so copies there are left unopened.
can_open() {
  case "$1" in
    /Applications/* | "$HOME/Applications"/* | "$APP_DIR"/*) return 0 ;;
  esac
  is_running_bundle "$1"
}

# Prints the other copies of Voxy on this Mac (Spotlight, plus running ones),
# except the install destination, the downloaded copy and this installer's
# hidden copies.
other_copies() {
  {
    mdfind "kMDItemCFBundleIdentifier == '$BUNDLE_ID'" 2>/dev/null || true
    if [ -n "$RUNNING" ]; then printf '%s\n' "$RUNNING" | cut -f 2; fi
  } | sort -u | while IFS= read -r path; do
    [ -n "$path" ] || continue
    [ "$path" != "$DEST" ] || continue
    case "$path" in "$TMP_DIR"/* | "$APP_DIR"/.Voxy.app.*) continue ;; esac
    # A Spotlight entry for a copy that was just deleted.
    if can_open "$path" && [ ! -d "$path" ]; then continue; fi
    printf '%s\n' "$path"
  done
}

# Sets RESET_TCC (0/1) and RESET_REASON. Needs NEW_DR, OLD_*, RUNNING, OTHERS.
decide_tcc_reset() {
  RESET_TCC=0
  RESET_REASON=""
  local path dr
  if [ -n "$OLD_VERSION_PATH" ]; then
    if [ -z "$NEW_DR" ]; then
      RESET_TCC=1
      RESET_REASON="the new version is not signed"
    elif [ -z "$OLD_DR" ]; then
      RESET_TCC=1
      RESET_REASON="the signature of the installed copy could not be read"
    elif [ "$OLD_DR" != "$NEW_DR" ]; then
      RESET_TCC=1
      RESET_REASON="this version has a different code signature than the one you had"
    fi
    [ "$RESET_TCC" = "0" ] || return 0
  fi
  # A first install, or the same signature as before: macOS may still have
  # tied the permissions to another copy of Voxy.
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    if can_open "$path"; then
      dr=$(designated_requirement "$path") || dr=""
      if [ -n "$NEW_DR" ] && [ "$dr" = "$NEW_DR" ]; then continue; fi
      RESET_REASON="another copy of Voxy with a different code signature is on this Mac ($path), and macOS may have tied the permissions to it"
    else
      RESET_REASON="another copy of Voxy is on this Mac ($path), and macOS may have tied the permissions to it"
    fi
    RESET_TCC=1
    return 0
  done <<EOF
$OTHERS
EOF
}

# ------------------------------------------------------------ download

CURL_COMMON=(--proto '=https' --proto-redir '=https' --connect-timeout 20 --retry 3 --retry-delay 2)
# A download that stalls (Wi-Fi hand-off, a captive proxy) fails after a
# minute below 1 KB/s, and --retry starts it again.
CURL_DOWNLOAD=(--speed-limit 1024 --speed-time 60)
AUTH=()
USE_TOKEN=0

# Fetches https://api.github.com/repos/$REPO/releases/latest into $1 and sets
# API_CODE to the HTTP status (000 when GitHub could not be reached).
fetch_latest_json() {
  API_CODE=$(curl -sS -L "${CURL_COMMON[@]}" --max-time 60 \
    -H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28' \
    ${AUTH[@]+"${AUTH[@]}"} -o "$1" -w '%{http_code}' \
    "https://api.github.com/repos/$REPO/releases/latest") || API_CODE="000"
}

# Releases are made only from tags like v1.2.3 (the release workflow refuses
# anything else, and a tag ruleset guards them), so a latest release with any
# other tag was not made by it: stop instead of installing it. This also keeps
# odd characters out of the URLs built from the tag below.
check_release_tag() {
  local re='^v[0-9]+\.[0-9]+\.[0-9]+$' shown
  if [[ ${RELEASE_TAG:-} =~ $re ]]; then
    return 0
  fi
  shown=$(printf '%s' "${RELEASE_TAG:-}" | LC_ALL=C tr -cd 'A-Za-z0-9._-' | cut -c1-40)
  die "The latest release has an unexpected tag (${shown:-none}), so the Voxy release workflow did not make it. Not installing it. See https://github.com/$REPO/releases"
}

download_release() {
  ARCHIVE="$TMP_DIR/Voxy.app.tar.gz"
  if [ -n "${VOXY_ARCHIVE:-}" ]; then
    [ -f "$VOXY_ARCHIVE" ] || die "VOXY_ARCHIVE: no such file: $VOXY_ARCHIVE"
    testlog "using the local archive $VOXY_ARCHIVE instead of downloading"
    cp "$VOXY_ARCHIVE" "$ARCHIVE" || die "Could not copy $VOXY_ARCHIVE"
    RELEASE_TAG="local"
    return 0
  fi

  local json="$TMP_DIR/release.json" code url="" api_url="" digest="" name
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    # From a file, so the token does not show up in the process list.
    printf 'Authorization: Bearer %s\n' "$GITHUB_TOKEN" >"$TMP_DIR/auth-header"
    AUTH=(-H "@$TMP_DIR/auth-header")
    USE_TOKEN=1
  fi

  step "Looking up the latest Voxy release"
  fetch_latest_json "$json"
  if [ "$API_CODE" = "401" ] && [ "$USE_TOKEN" = "1" ]; then
    # A public repository needs no token; an expired one must not block.
    warn "GitHub rejected GITHUB_TOKEN (HTTP 401), so the installer goes on without it."
    AUTH=()
    USE_TOKEN=0
    fetch_latest_json "$json"
  fi
  code=$API_CODE

  case "$code" in
    200)
      local urls api_urls digests idx n
      RELEASE_TAG=$(json_values tag_name "$json" | sed -n 1p)
      check_release_tag
      urls=$(json_values browser_download_url "$json")
      idx=$(printf '%s\n' "$urls" | awk -v s="$ASSET_SUFFIX" \
        'length($0) >= length(s) && substr($0, length($0) - length(s) + 1) == s { print NR; exit }')
      if [ -z "$idx" ]; then
        die "The latest release (${RELEASE_TAG:-unknown}) has no macOS app (*$ASSET_SUFFIX). See https://github.com/$REPO/releases"
      fi
      url=$(nth_line "$urls" "$idx")
      # The asset's API URL and digest sit in the same asset object; pair them
      # by position, and only when the counts match.
      api_urls=$(grep -o '"url"[[:space:]]*:[[:space:]]*"https://api\.github\.com/repos/[^"]*/releases/assets/[0-9]*"' "$json" |
        sed 's/^"url"[[:space:]]*:[[:space:]]*"//; s/"$//' || true)
      digests=$(grep -oE '"digest"[[:space:]]*:[[:space:]]*(null|"[^"]*")' "$json" |
        sed -E 's/^"digest"[[:space:]]*:[[:space:]]*//; s/"//g' || true)
      n=$(count_lines "$urls")
      if [ "$(count_lines "$api_urls")" = "$n" ]; then api_url=$(nth_line "$api_urls" "$idx"); fi
      if [ "$(count_lines "$digests")" = "$n" ]; then digest=$(nth_line "$digests" "$idx"); fi
      ;;
    404)
      die "No published Voxy release was found at https://github.com/$REPO/releases (if the repository is private, set GITHUB_TOKEN)."
      ;;
    403 | 429)
      # Without the API: github.com/<repo>/releases/latest redirects to
      # .../releases/tag/<tag>, and the release's asset list is an HTML page
      # that does not count against the API rate limit.
      local location page="$TMP_DIR/assets.html" href="" suffix_re
      location=$(curl -sS "${CURL_COMMON[@]}" --max-time 60 -o /dev/null -w '%{redirect_url}' \
        "https://github.com/$REPO/releases/latest") || location=""
      case "$location" in
        */releases/tag/?*)
          RELEASE_TAG=${location##*/releases/tag/}
          check_release_tag
          ;;
        *)
          if [ "$USE_TOKEN" = "1" ]; then
            die "The GitHub API refused the request (HTTP $code) although GITHUB_TOKEN is set, and the latest release could not be found without the API. Check the token, or unset it and try again."
          fi
          die "The GitHub API rate limit is reached, and the latest release could not be found without it. Try again in an hour, or set GITHUB_TOKEN."
          ;;
      esac
      suffix_re=$(printf '%s' "$ASSET_SUFFIX" | sed 's/\./\\./g')
      if curl -fsSL "${CURL_COMMON[@]}" --max-time 60 -o "$page" \
        "https://github.com/$REPO/releases/expanded_assets/$RELEASE_TAG" 2>/dev/null; then
        href=$(grep -o "href=\"/[^\"]*/releases/download/[^\"]*$suffix_re\"" "$page" |
          sed -n '1{s/^href="//;s/"$//;p;}') || href=""
      fi
      if [ -n "$href" ]; then
        url="https://github.com$href"
      else
        # The name the release workflow gives the asset.
        url="https://github.com/$REPO/releases/download/$RELEASE_TAG/Voxy_${RELEASE_TAG#v}_$ASSET_SUFFIX"
      fi
      if [ "$USE_TOKEN" = "1" ]; then
        warn "The GitHub API refused the request (HTTP $code); downloading $RELEASE_TAG without it."
      else
        warn "The GitHub API rate limit is reached; downloading $RELEASE_TAG without it."
      fi
      ;;
    000)
      die "Could not reach api.github.com. Check your internet connection and try again."
      ;;
    *)
      die "The GitHub API answered with HTTP $code. Try again later."
      ;;
  esac

  name=${url##*/}
  step "Downloading $name (${RELEASE_TAG:-latest})"
  local -a progress=(-sS)
  if [ -t 2 ]; then progress=(--progress-bar); fi
  if [ "$USE_TOKEN" = "1" ] && [ -n "$api_url" ]; then
    # Works for private repositories too; curl drops the token on the
    # redirect to GitHub's download host.
    curl -fL "${CURL_COMMON[@]}" "${CURL_DOWNLOAD[@]}" "${progress[@]}" -H 'Accept: application/octet-stream' \
      "${AUTH[@]}" -o "$ARCHIVE" "$api_url" || die "The download failed: $api_url"
  else
    curl -fL "${CURL_COMMON[@]}" "${CURL_DOWNLOAD[@]}" "${progress[@]}" -o "$ARCHIVE" "$url" ||
      die "The download failed: $url"
  fi

  case "$digest" in
    sha256:*)
      local actual
      if ! actual=$(sha256_of "$ARCHIVE"); then
        warn "Could not compute the SHA-256 of the download; skipping that check."
      elif [ "$actual" != "${digest#sha256:}" ]; then
        die "The downloaded file is damaged (SHA-256 mismatch). Run the installer again."
      else
        info "SHA-256 checked"
      fi
      ;;
  esac
}

# A release is always a sealed, signed bundle, so a bad signature means a
# damaged or modified download: stop. Local test archives (VOXY_ARCHIVE) are
# often unsealed dev builds, so for them it is only a warning.
reject_app() {
  if [ -n "${VOXY_ARCHIVE:-}" ]; then
    warn "$1 Installing it anyway, because it is a local archive (VOXY_ARCHIVE)."
  else
    die "$1 Not installing it. $2"
  fi
}

# Under the hardened runtime, macOS gives the microphone only to apps with the
# audio-input entitlement; without it Voxy would record silence.
check_microphone_entitlement() {
  local info flags ents="$TMP_DIR/entitlements.plist" value
  info=$(codesign -dv "$NEW_APP" 2>&1) || info=""
  flags=$(printf '%s\n' "$info" | sed -n 's/^CodeDirectory .*flags=\(0x[0-9a-fA-F]*([^)]*)\).*/\1/p')
  case "$flags" in *runtime*) ;; *) return 0 ;; esac
  codesign -d --entitlements - --xml "$NEW_APP" >"$ents" 2>/dev/null || true
  value=$(plist_get "$ents" "com.apple.security.device.audio-input") || value=""
  if [ "$value" != "true" ]; then
    reject_app "This build of Voxy lacks the microphone entitlement (com.apple.security.device.audio-input), so it could not record." \
      "Please report this at https://github.com/$REPO/issues"
  fi
}

# Extracts the archive and checks the app. Sets NEW_APP, NEW_EXE, NEW_VERSION,
# NEW_DR.
extract_and_check() {
  local list="$TMP_DIR/list.txt" out="$TMP_DIR/extract" id min
  step "Checking the app"
  if ! tar -tzf "$ARCHIVE" >"$list" 2>/dev/null; then
    die "The downloaded file is not a valid .tar.gz archive (damaged or incomplete download?). Run the installer again."
  fi
  if ! grep -qE "^(\./)?Voxy\.app/Contents/Info\.plist\$" "$list"; then
    die "The archive does not contain $APP_NAME."
  fi
  mkdir -p "$out"
  tar -xzf "$ARCHIVE" -C "$out" 2>/dev/null || die "Could not extract the archive (damaged or incomplete download?)."
  NEW_APP="$out/$APP_NAME"

  id=$(plist_get "$NEW_APP/Contents/Info.plist" CFBundleIdentifier) || id=""
  if [ "$id" != "$BUNDLE_ID" ]; then
    die "The app in the archive has bundle identifier '${id:-none}', expected $BUNDLE_ID. Not installing it."
  fi
  NEW_EXE=$(plist_get "$NEW_APP/Contents/Info.plist" CFBundleExecutable) || NEW_EXE=""
  if [ -z "$NEW_EXE" ] || [ ! -x "$NEW_APP/Contents/MacOS/$NEW_EXE" ]; then
    die "The app in the archive has no executable. Not installing it."
  fi
  min=$(plist_get "$NEW_APP/Contents/Info.plist" LSMinimumSystemVersion) || min=""
  if [ -n "$min" ] && ! version_ge "$MACOS_VERSION" "$min"; then
    die "This Voxy release needs macOS $min or later, and this Mac has macOS $MACOS_VERSION."
  fi
  NEW_VERSION=$(plist_get "$NEW_APP/Contents/Info.plist" CFBundleShortVersionString) || NEW_VERSION="?"

  local again="Run the installer again; if this keeps happening, report it at https://github.com/$REPO/issues"
  NEW_DR=$(designated_requirement "$NEW_APP") || NEW_DR=""
  if [ -z "$NEW_DR" ]; then
    reject_app "The app in the archive is not code-signed." "$again"
  elif ! codesign --verify --strict "$NEW_APP" >/dev/null 2>&1; then
    reject_app "The app's code signature does not verify: the download is damaged or was modified." "$again"
  else
    check_microphone_entitlement
  fi
  info "Voxy $NEW_VERSION, $BUNDLE_ID"
}

# ------------------------------------------------------------- install

choose_app_dir() {
  if [ -n "${VOXY_APP_DIR:-}" ]; then
    APP_DIR=$VOXY_APP_DIR
    mkdir -p "$APP_DIR" || die "Could not create $APP_DIR"
  elif [ -w /Applications ]; then
    APP_DIR=/Applications
  else
    APP_DIR="$HOME/Applications"
    mkdir -p "$APP_DIR" || die "Could not create $APP_DIR"
    warn "Your account cannot write to /Applications, so Voxy goes to $APP_DIR."
    if [ -d "/Applications/$APP_NAME" ] && ! foreign_id "/Applications/$APP_NAME" >/dev/null; then
      warn "An older Voxy is in /Applications, and your account cannot remove it. Ask an administrator to delete it, so only one copy is left."
    fi
  fi
  DEST="$APP_DIR/$APP_NAME"
}

APP_MANAGEMENT_HINT="If macOS said that your terminal app was prevented from modifying apps, allow it in System Settings > Privacy & Security > App Management, then run the installer again."

# Deletes a leftover copy of Voxy. If that fails (files owned by another
# account, or App Management), moves it to the Trash, where Finder shows it.
discard_bundle() {
  local path=$1 trash
  rm -rf "$path" 2>/dev/null || true
  [ -e "$path" ] || [ -L "$path" ] || return 0
  trash="${VOXY_DATA_ROOT:-$HOME}/.Trash/Voxy $(date '+%Y-%m-%d %H.%M.%S').app"
  if mv "$path" "$trash" 2>/dev/null; then
    warn "Could not delete the previous version of Voxy, so it is in the Trash now: $trash"
  else
    warn "Could not delete the previous version of Voxy at $path (hidden in Finder; press Cmd-Shift-. to show it). Delete it yourself."
  fi
}

# An installer that was killed between its two renames, or could not delete
# the old version, leaves .Voxy.app.old.<PID> or .Voxy.app.new.<PID> in the
# Applications folder, hidden from Finder. Puts a lost Voxy.app back and
# deletes the rest. Runs under the lock, so no other installer is using them.
clean_leftovers() {
  local path pid
  for path in "$APP_DIR"/.Voxy.app.old.* "$APP_DIR"/.Voxy.app.new.*; do
    [ -e "$path" ] || [ -L "$path" ] || continue
    pid=${path##*.}
    if [ "$pid" != "$$" ] && kill -0 "$pid" 2>/dev/null; then continue; fi
    case "$path" in
      */.Voxy.app.old.*)
        if [ ! -e "$DEST" ] && [ ! -L "$DEST" ] && mv "$path" "$DEST" 2>/dev/null; then
          warn "An earlier update was interrupted; put the previous $APP_NAME back."
          continue
        fi
        ;;
    esac
    discard_bundle "$path"
  done
}

# Copies the new app next to the installed one, under a hidden name. Runs
# before Voxy is stopped, so a failed copy leaves the running app alone.
stage_bundle() {
  STAGING_PATH="$APP_DIR/.Voxy.app.new.$$"
  rm -rf "$STAGING_PATH"
  ditto "$NEW_APP" "$STAGING_PATH" 2>/dev/null ||
    die "Could not copy Voxy to $APP_DIR (is the disk full?). $APP_MANAGEMENT_HINT"
}

# Swaps the staged app in with two renames, so Voxy.app is either the old or
# the new version, never half-copied.
swap_bundle() {
  local staged=${STAGING_PATH##*/} old
  if [ -e "$DEST" ] || [ -L "$DEST" ]; then
    BACKUP_PATH="$APP_DIR/.Voxy.app.old.$$"
    rm -rf "$BACKUP_PATH"
    mv "$DEST" "$BACKUP_PATH" 2>/dev/null || die "Could not replace $DEST. $APP_MANAGEMENT_HINT"
  fi
  # If Voxy.app came back in the meantime (a copy from the .dmg), mv would
  # move the new app into it instead of replacing it.
  if [ -e "$DEST" ] || [ -L "$DEST" ]; then
    die "Another $APP_NAME appeared in $APP_DIR during the install. Run the installer again."
  fi
  mv "$STAGING_PATH" "$DEST" 2>/dev/null || die "Could not move Voxy into $APP_DIR. $APP_MANAGEMENT_HINT"
  if [ -e "$DEST/$staged" ]; then
    rm -rf "${DEST:?}/$staged" 2>/dev/null || true
    die "Another $APP_NAME appeared in $APP_DIR during the install. Run the installer again."
  fi
  STAGING_PATH=""
  if [ -n "$BACKUP_PATH" ]; then
    old=$BACKUP_PATH
    BACKUP_PATH=""
    discard_bundle "$old"
  fi
  xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true
}

# Launch at login is a LaunchAgent that the app writes with the path of the
# copy that turned it on. If it starts Voxy from the other Applications folder
# (or an older executable name), point it at the new copy; for any other copy,
# say how to switch it.
update_launch_agent() {
  local file="${VOXY_DATA_ROOT:-$HOME}/Library/LaunchAgents/Voxy.plist" target exe
  # A test install into VOXY_APP_DIR leaves the real login item alone.
  if [ -n "${VOXY_APP_DIR:-}" ] && [ -z "${VOXY_DATA_ROOT:-}" ]; then return 0; fi
  [ -f "$file" ] || return 0
  target=$(plist_get "$file" "ProgramArguments:0") || return 0
  case "$target" in
    *.app/Contents/MacOS/voxy | *.app/Contents/MacOS/voice) ;;
    *) return 0 ;;
  esac
  exe="$DEST/Contents/MacOS/$NEW_EXE"
  [ "$target" != "$exe" ] || return 0
  case "${target%/Contents/MacOS/*}" in
    "/Applications/$APP_NAME" | "$HOME/Applications/$APP_NAME" | "$DEST")
      if "$PLISTBUDDY" -c "Set :ProgramArguments:0 $exe" "$file" >/dev/null 2>&1; then
        info "Launch at login now starts $DEST."
      else
        warn "Launch at login still starts $target. Turn Launch at login off and on again in Voxy's Settings > General."
      fi
      ;;
    *)
      warn "Launch at login starts another copy of Voxy ($target). To start this one instead, turn Launch at login off and on again in Voxy's Settings > General."
      ;;
  esac
}

do_install() {
  local id
  check_system install
  choose_app_dir
  if [ -e "$DEST" ] || [ -L "$DEST" ]; then
    if id=$(foreign_id "$DEST"); then
      die "$DEST belongs to another app ($id). Move it elsewhere and run the installer again."
    fi
  fi
  clean_leftovers
  make_temp_dir
  download_release
  extract_and_check

  OLD_VERSION_PATH=""
  OLD_VERSION=""
  OLD_DR=""
  if [ -d "$DEST" ]; then
    OLD_VERSION_PATH=$DEST
    OLD_VERSION=$(plist_get "$DEST/Contents/Info.plist" CFBundleShortVersionString) || OLD_VERSION="?"
    OLD_DR=$(designated_requirement "$DEST") || OLD_DR=""
  fi

  RUNNING=$(list_running) || RUNNING=""
  OTHERS=$(other_copies) || OTHERS=""
  decide_tcc_reset

  if [ -n "$OLD_VERSION_PATH" ]; then
    step "Updating $DEST ($OLD_VERSION -> $NEW_VERSION)"
  else
    step "Installing to $DEST"
  fi
  stage_bundle
  stop_running
  RESTART_PATH=$STOPPED_FIRST
  swap_bundle
  RESTART_PATH=""

  if [ "$RESET_TCC" = "1" ]; then
    step "Resetting Accessibility and Input Monitoring"
    info "macOS will ask for both permissions again, because $RESET_REASON."
    if ! is_set "${VOXY_NO_TCC:-}"; then register_app "$DEST"; fi
    if ! reset_tcc Accessibility ListenEvent; then
      warn "Could not reset:$TCC_FAILED. If the hotkey does nothing, remove Voxy from Accessibility and Input Monitoring in System Settings > Privacy & Security (the - button), then allow it again."
    fi
  elif [ -n "$OLD_VERSION_PATH" ]; then
    info "The code signature is unchanged, so Voxy keeps its permissions. If the"
    info "hotkey does nothing, run: $INSTALL_CMD -s -- --reset-permissions"
  fi
  update_launch_agent

  launch_app "$DEST" || warn "Could not open Voxy. Open it from $APP_DIR yourself."

  echo
  if [ -n "$OLD_VERSION_PATH" ] && [ "$OLD_VERSION" = "$NEW_VERSION" ]; then
    ok "Voxy $NEW_VERSION is reinstalled."
  elif [ -n "$OLD_VERSION_PATH" ]; then
    ok "Voxy is updated to $NEW_VERSION."
  else
    ok "Voxy $NEW_VERSION is installed in $APP_DIR."
  fi
  if [ -z "$OLD_VERSION_PATH" ] || [ "$RESET_TCC" = "1" ]; then
    info "- When macOS asks, allow Accessibility and Input Monitoring"
    info "  (System Settings > Privacy & Security). The Settings window shows"
    info "  which one is still missing; no restart is needed."
  fi
  if [ -z "$OLD_VERSION_PATH" ]; then
    info "- The default speech model (670 MB) downloads on the first launch;"
    info "  progress is in Settings > Model."
  fi
  info "- Dictate: hold Right ⌘, speak, release. The text is pasted at the cursor."
  info "- Settings: menu bar icon > Settings. To update, run the same command again."
  if [ -n "$OTHERS" ]; then
    info "- Other copies of Voxy are on this Mac. Only one copy can run at a time,"
    info "  so delete the ones you no longer need:"
    printf '%s\n' "$OTHERS" | while IFS= read -r line; do info "    $line"; done
  fi
}

# ------------------------------------------------- reset permissions only

do_reset_permissions() {
  local dir app="" pid bundle
  check_system reset
  if [ -n "${VOXY_APP_DIR:-}" ]; then set -- "$VOXY_APP_DIR"; else set -- /Applications "$HOME/Applications"; fi
  for dir in "$@"; do
    if [ -d "$dir/$APP_NAME" ] && ! foreign_id "$dir/$APP_NAME" >/dev/null; then
      app="$dir/$APP_NAME"
      break
    fi
  done
  RUNNING=$(list_running) || RUNNING=""
  if [ -z "$app" ] && [ -n "$RUNNING" ]; then
    IFS=$TAB read -r pid bundle <<EOF || true
$RUNNING
EOF
    app=$bundle
  fi
  [ -n "$app" ] || die "Voxy is not installed. To install it, run: $INSTALL_CMD"

  step "Resetting Accessibility and Input Monitoring for Voxy"
  stop_running
  if ! is_set "${VOXY_NO_TCC:-}"; then register_app "$app"; fi
  if ! reset_tcc Accessibility ListenEvent; then
    launch_app "$app" || true
    die "Could not reset:$TCC_FAILED. Quit Voxy, remove it from Accessibility and Input Monitoring in System Settings > Privacy & Security (the - button), then open it and allow both again."
  fi
  launch_app "$app" || warn "Could not open Voxy. Open $app yourself."
  echo
  ok "Done. When macOS asks, allow Accessibility and Input Monitoring for Voxy"
  info "(System Settings > Privacy & Security); no restart is needed."
}

# ----------------------------------------------------------- uninstall

REMOVED=""

remove_path() {
  local path=$1
  if [ -e "$path" ] || [ -L "$path" ]; then
    if rm -rf "$path" 2>/dev/null; then
      REMOVED="$REMOVED$path
"
    else
      warn "Could not delete $path"
    fi
  fi
}

# The autostart plugin writes ~/Library/LaunchAgents/<product name>.plist:
# Voxy.plist now, Voice.plist or Typely.plist in older builds. Only files that
# start this app's executable are touched.
remove_launch_agents() {
  local home=$1 name file label
  for name in Voxy Voice Typely; do
    file="$home/Library/LaunchAgents/$name.plist"
    [ -f "$file" ] || continue
    if ! grep -qE '\.app/Contents/MacOS/(voxy|voice)<' "$file"; then
      warn "Left $file alone: it does not start Voxy."
      continue
    fi
    label=$name
    if [ -n "${VOXY_DATA_ROOT:-}" ]; then
      testlog "would run: launchctl bootout gui/$(id -u)/$label"
    else
      launchctl bootout "gui/$(id -u)/$label" >/dev/null 2>&1 || true
    fi
    remove_path "$file"
  done
}

remove_keychain_items() {
  if [ -n "${VOXY_DATA_ROOT:-}" ]; then
    testlog "would delete Keychain items of service $BUNDLE_ID"
    return 0
  fi
  local n=0
  while [ "$n" -lt 50 ] && security delete-generic-password -s "$BUNDLE_ID" >/dev/null 2>&1; do
    n=$((n + 1))
  done
  if [ "$n" -gt 0 ]; then
    REMOVED="${REMOVED}Keychain: $n item(s) of service $BUNDLE_ID
"
  fi
}

remove_app_at() {
  local app="$1/$APP_NAME" id
  [ -e "$app" ] || return 0
  if id=$(foreign_id "$app"); then
    warn "Left $app alone: its bundle identifier is $id, not $BUNDLE_ID."
    return 0
  fi
  remove_path "$app"
}

do_uninstall() {
  local keep=$1 home dir lib
  check_system uninstall
  home=${VOXY_DATA_ROOT:-$HOME}
  lib="$home/Library"

  step "Uninstalling Voxy"
  RUNNING=$(list_running) || RUNNING=""
  stop_running

  remove_launch_agents "$home"

  # Before the app is deleted, while LaunchServices still knows the bundle
  # (tccutil fails for a bundle identifier it cannot resolve).
  local tcc_ok=0
  if reset_tcc All; then tcc_ok=1; fi

  if [ -n "${VOXY_APP_DIR:-}" ]; then
    remove_app_at "$VOXY_APP_DIR"
  else
    for dir in /Applications "$home/Applications"; do remove_app_at "$dir"; done
  fi

  if [ "$keep" = "1" ]; then
    info "Keeping models, settings and API keys ($lib/Application Support/$BUNDLE_ID, Keychain)."
  else
    remove_path "$lib/Application Support/$BUNDLE_ID"
    remove_keychain_items
  fi
  remove_path "$lib/Logs/Voxy.log"
  remove_path "$lib/WebKit/$BUNDLE_ID"
  remove_path "$lib/Caches/$BUNDLE_ID"
  remove_path "$lib/HTTPStorages/$BUNDLE_ID"
  remove_path "$lib/Saved Application State/$BUNDLE_ID.savedState"
  remove_path "$lib/Preferences/$BUNDLE_ID.plist"

  echo
  if [ -n "$REMOVED" ]; then
    ok "Voxy is uninstalled. Removed:"
    printf '%s' "$REMOVED" | while IFS= read -r line; do info "$line"; done
  else
    ok "Voxy was not installed; nothing to remove."
  fi
  if is_set "${VOXY_NO_TCC:-}"; then
    :
  elif [ "$tcc_ok" = "1" ]; then
    info "Privacy entries (Accessibility, Input Monitoring, Microphone) are reset."
  else
    warn "Could not reset Voxy's privacy entries. If Voxy is still listed in System Settings > Privacy & Security, remove it there with the - button."
  fi

  local others
  others=$(mdfind "kMDItemCFBundleIdentifier == '$BUNDLE_ID'" 2>/dev/null || true)
  if [ -n "$others" ]; then
    info "Other copies are still on this Mac:"
    printf '%s\n' "$others" | while IFS= read -r line; do info "  $line"; done
  fi
}

# ---------------------------------------------------------------- main

main() {
  # Commands must not read the rest of the script from the pipe.
  exec </dev/null
  setup_colors
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap 'exit 129' HUP

  local uninstall=0 reset=0 keep=0 arg
  for arg in "$@"; do
    case "$arg" in
      --uninstall) uninstall=1 ;;
      --reset-permissions) reset=1 ;;
      --keep-data) keep=1 ;;
      -h | --help)
        usage
        return 0
        ;;
      *)
        usage >&2
        die "Unknown option: $arg" 2
        ;;
    esac
  done
  if [ "$keep" = "1" ] && [ "$uninstall" != "1" ]; then
    die "--keep-data only works with --uninstall." 2
  fi
  if [ "$reset" = "1" ] && [ "$uninstall" = "1" ]; then
    die "--reset-permissions and --uninstall can't be used together (--uninstall resets the permissions anyway)." 2
  fi
  if [ "$(id -u)" = "0" ]; then
    die "Run the installer as yourself, without sudo: permissions, settings and the login item belong to your user."
  fi
  take_lock

  if [ "$uninstall" = "1" ]; then
    do_uninstall "$keep"
  elif [ "$reset" = "1" ]; then
    do_reset_permissions
  else
    do_install
  fi
}

main "$@"
