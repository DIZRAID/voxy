#!/usr/bin/env bash
# Downloads the sherpa-onnx static libraries that sherpa-onnx-sys links into
# Voxy, checks the archive against src-tauri/sherpa-onnx.sha256, unpacks it
# and exports SHERPA_ONNX_LIB_DIR (through $GITHUB_ENV) for the next steps.
# With SHERPA_ONNX_LIB_DIR set, the sherpa-onnx-sys build script uses these
# libraries and downloads nothing itself (it does not check any hash).
#
# Usage (GitHub Actions, bash on macOS or Windows):
#   bash .github/scripts/fetch-sherpa-onnx.sh osx-arm64-static-lib
#   bash .github/scripts/fetch-sherpa-onnx.sh win-x64-static-MT-Release-lib
# The archive name follows archive_name() in sherpa-onnx-sys's build.rs
# (default "static" feature): sherpa-onnx-v<version>-<platform>.tar.bz2.
set -euo pipefail

platform=${1:?usage: fetch-sherpa-onnx.sh <platform, e.g. osx-arm64-static-lib>}
root=$(cd "$(dirname "$0")/../.." && pwd)
lock="$root/src-tauri/Cargo.lock"
pins="$root/src-tauri/sherpa-onnx.sha256"

version=$(awk -F'"' '/^name = "sherpa-onnx-sys"/ { getline; print $2; exit }' "$lock")
if [ -z "$version" ]; then
  echo "::error::sherpa-onnx-sys not found in src-tauri/Cargo.lock"
  exit 1
fi
name="sherpa-onnx-v${version}-${platform}"
file="$name.tar.bz2"
url="https://github.com/k2-fsa/sherpa-onnx/releases/download/v${version}/${file}"

dir="${RUNNER_TEMP:?}/sherpa-onnx"
mkdir -p "$dir"
cd "$dir"

# The pinned line for exactly this file, in "<sha256>  <name>" form.
awk -v f="$file" '$2 == f && $1 ~ /^[0-9a-f]+$/ && length($1) == 64 { print; n++ } END { exit n == 1 ? 0 : 1 }' \
  "$pins" >expected.sha256 || {
  echo "::error::No single pinned SHA-256 for $file in src-tauri/sherpa-onnx.sha256. Add it deliberately (see the comment in that file) when bumping sherpa-onnx-sys."
  exit 1
}

echo "Downloading $url"
curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 -o "$file" "$url"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c expected.sha256
else
  shasum -a 256 -c expected.sha256
fi || {
  echo "::error::$file does not match the SHA-256 pinned in src-tauri/sherpa-onnx.sha256. The upstream release asset changed; do not build with it."
  exit 1
}

if [ "${RUNNER_OS:-}" = "Windows" ]; then
  # Windows' own bsdtar unpacks .tar.bz2 without an external bzip2.
  /c/Windows/System32/tar.exe -xf "$file"
else
  tar -xf "$file"
fi
rm "$file" expected.sha256

libdir="$PWD/$name/lib"
if [ ! -d "$libdir" ]; then
  echo "::error::$file has no $name/lib directory"
  exit 1
fi
ls -l "$libdir"
if [ "${RUNNER_OS:-}" = "Windows" ]; then
  libdir=$(cygpath -w "$libdir")
fi
echo "SHERPA_ONNX_LIB_DIR=$libdir" >>"${GITHUB_ENV:?}"
