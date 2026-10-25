#!/usr/bin/env bash
# Type-check Voxy for Windows or macOS from a Linux machine, without either OS.
#
#   scripts/cross-check.sh win check --all-targets
#   scripts/cross-check.sh win clippy --all-targets
#   scripts/cross-check.sh mac check --all-targets
#   scripts/cross-check.sh mac clippy --all-targets
#
# Nothing is linked, so this proves that the cfg(windows) / cfg(macos) code
# compiles and passes clippy, not that the app runs. `cargo test --no-run`
# cannot work here (it links), and a native Linux build is impossible anyway
# (Tauri would need webkit2gtk, and Voxy only targets macOS and Windows).
#
# How: zig (pip package `ziglang`) is the C compiler for the few crates with C
# code (ring, ...), windres is a stub (tauri-build compiles the exe's icon and
# manifest resource, which a type check doesn't need), and SHERPA_ONNX_LIB_DIR
# points at an empty folder so sherpa-onnx-sys downloads nothing.
#
# Windows is checked as x86_64-pc-windows-gnu: the same cfg(windows) Rust code
# as the MSVC target that releases use, but MSVC-only linker flags (build.rs)
# are not exercised. macOS is checked as aarch64-apple-darwin.
#
# One-time setup: `pip install ziglang` (add --break-system-packages on
# distros that need it) and the Rust std for both targets (this script runs
# `rustup target add` for you).
#
# Build output goes to src-tauri/target/xwin and src-tauri/target/xmac of the
# main checkout, shared by its git worktrees (dependencies compile once).
# VOXY_CROSS_TARGET_DIR overrides the parent folder.
set -euo pipefail

usage() {
    echo "usage: $0 win|mac [cargo subcommand, default: check] [cargo args...]" >&2
    exit 2
}

[ $# -ge 1 ] || usage
platform=$1
shift
case "$platform" in
    win) target=x86_64-pc-windows-gnu zig_target=x86_64-windows-gnu ;;
    mac) target=aarch64-apple-darwin zig_target=aarch64-macos ;;
    *) usage ;;
esac
subcommand=${1:-check}
[ $# -ge 1 ] && shift

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
common_git=$(git -C "$repo" rev-parse --path-format=absolute --git-common-dir)
main_repo=$(dirname "$common_git")
target_root=${VOXY_CROSS_TARGET_DIR:-$main_repo/src-tauri/target}

if ! python3 -m ziglang version >/dev/null 2>&1; then
    echo "zig not found: run 'pip install ziglang' first" >&2
    exit 1
fi
if ! rustup target list --installed | grep -qx "$target"; then
    rustup target add "$target"
fi

tools=${XDG_CACHE_HOME:-$HOME/.cache}/voxy-cross
mkdir -p "$tools/bin" "$tools/sherpa-empty"

# C compiler wrapper: zig picks the target itself, so drop the target and
# deployment flags that cc-rs adds for clang.
cat >"$tools/bin/zcc-$platform" <<EOF
#!/usr/bin/env bash
out=()
skip=0
for a in "\$@"; do
    if [ \$skip = 1 ]; then skip=0; continue; fi
    case "\$a" in
        --target=*|-mmacosx-version-min=*) ;;
        -target|-arch) skip=1 ;;
        *) out+=("\$a") ;;
    esac
done
exec python3 -m ziglang cc -target $zig_target "\${out[@]}"
EOF
printf '#!/bin/sh\nexec python3 -m ziglang ar "$@"\n' >"$tools/bin/zar"
# windres stub: create the requested output file and nothing else.
cat >"$tools/bin/x86_64-w64-mingw32-windres" <<'EOF'
#!/usr/bin/env bash
out=""
prev=""
for a in "$@"; do
    case "$a" in
        --output=*) out=${a#--output=} ;;
        -o?*) out=${a#-o} ;;
    esac
    if [ "$prev" = "--output" ] || [ "$prev" = "-o" ]; then out=$a; fi
    prev=$a
done
[ -n "$out" ] && : >"$out"
exit 0
EOF
chmod +x "$tools/bin/"*

env_target=$(echo "$target" | tr '-' '_')
export PATH="$tools/bin:$PATH"
export "CC_$env_target=$tools/bin/zcc-$platform"
export "AR_$env_target=$tools/bin/zar"
export SHERPA_ONNX_LIB_DIR="$tools/sherpa-empty"

cd "$repo/src-tauri"
exec cargo "$subcommand" --locked --target "$target" \
    --target-dir "$target_root/x$platform" "$@"
