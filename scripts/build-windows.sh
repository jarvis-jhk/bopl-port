#!/bin/bash
# Cross-compiles bopl-port.exe for x86_64 Windows (x86_64-pc-windows-gnu,
# llvm-mingw from scripts/fetch-windows-toolchain.sh) and stages it with
# SDL2.dll in target/dist/windows/.
# Usage: scripts/build-windows.sh [--debug] [--xcheck]
#   --xcheck  also build the cross-platform determinism checker
#             (scripts/xcheck -> target/dist/windows/bopl-xcheck.exe)
# Build output: target/windows/ (BOPL_WIN_TARGET_DIR), separate from the
# Linux builds in target/.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
[ -x tools/windows/llvm-mingw/bin/x86_64-w64-mingw32-clang ] || scripts/fetch-windows-toolchain.sh
source scripts/windows-env.sh
PROFILE=release; FLAG=--release; XCHECK=0
for a in "$@"; do
    case $a in
        --debug) PROFILE=debug; FLAG= ;;
        --xcheck) XCHECK=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
T=x86_64-pc-windows-gnu
cargo build $FLAG ${CI:+--locked} --target $T --target-dir "$BOPL_WIN_TARGET_DIR" -p bopl-port
OUT=target/dist/windows
mkdir -p "$OUT"
cp -f "$BOPL_WIN_TARGET_DIR/$T/$PROFILE/bopl-port.exe" "$OUT/"
cp -f tools/windows/sdl2/x86_64-w64-mingw32/bin/SDL2.dll "$OUT/"
if [ $XCHECK = 1 ]; then
    cargo build $FLAG --target $T --target-dir "$BOPL_WIN_TARGET_DIR" \
        --manifest-path scripts/xcheck/Cargo.toml
    cp -f "$BOPL_WIN_TARGET_DIR/$T/$PROFILE/bopl-xcheck.exe" "$OUT/"
fi
echo "staged: $OUT/"
ls -la "$OUT"
