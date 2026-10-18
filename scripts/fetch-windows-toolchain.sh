#!/bin/bash
# Fetches the Windows cross-compilation toolchain into tools/windows/
# (nothing outside the project):
#   - llvm-mingw (clang + lld + mingw-w64 runtime, self-contained, Linux host)
#   - the official SDL2 mingw development release (import library + SDL2.dll)
# Versions and SHA-256 digests are pinned (digests as published by GitHub
# for the release assets). Re-running is a no-op once everything is in place.
#
# Then build with:  scripts/build-windows.sh  (see docs/platform.md)
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"

LLVM_MINGW_VERSION=20261006
LLVM_MINGW_FILE="llvm-mingw-${LLVM_MINGW_VERSION}-msvcrt-ubuntu-22.04-x86_64.tar.xz"
LLVM_MINGW_URL="https://github.com/mstorsjo/llvm-mingw/releases/download/${LLVM_MINGW_VERSION}/${LLVM_MINGW_FILE}"
LLVM_MINGW_SHA256=9dd3bbff7f746937c99b38322c8b1fcab8054b7d8d31ec840b3457234dff7e52

SDL2_VERSION=2.32.10
SDL2_FILE="SDL2-devel-${SDL2_VERSION}-mingw.tar.gz"
SDL2_URL="https://github.com/libsdl-org/SDL/releases/download/release-${SDL2_VERSION}/${SDL2_FILE}"
SDL2_SHA256=83a5d74012311edc3c0d40ea6faecbe57ad692aa033fa5dc273cc937e3938ff2

W=tools/windows
mkdir -p "$W/downloads"

fetch() { # url file sha256
    local url=$1 file=$W/downloads/$2 sha=$3
    if [ ! -f "$file" ] || ! echo "$sha  $file" | sha256sum -c --status; then
        echo "downloading $2"
        curl -sSfL --retry 3 -o "$file.part" "$url"
        echo "$sha  $file.part" | sha256sum -c --status || { echo "SHA-256 mismatch for $2" >&2; rm -f "$file.part"; exit 1; }
        mv "$file.part" "$file"
    fi
}

# Each archive is unpacked into its own fresh directory, then moved into place.
unpack() { # file dest
    local file=$W/downloads/$1 dest=$2 tmp
    [ -d "$dest" ] && return
    tmp=$(mktemp -d "$W/unpack.XXXXXX")
    tar -xf "$file" -C "$tmp" --no-same-owner
    # Archives contain a single top-level directory.
    mv "$tmp"/*/ "$dest"
    rmdir "$tmp"
}

fetch "$LLVM_MINGW_URL" "$LLVM_MINGW_FILE" "$LLVM_MINGW_SHA256"
fetch "$SDL2_URL" "$SDL2_FILE" "$SDL2_SHA256"
unpack "$LLVM_MINGW_FILE" "$W/llvm-mingw"
unpack "$SDL2_FILE" "$W/sdl2"
echo "$LLVM_MINGW_VERSION" > "$W/llvm-mingw/.version"
echo "$SDL2_VERSION" > "$W/sdl2/.version"

# Rust's x86_64-pc-windows-gnu target links libgcc/libgcc_eh (GCC's runtime);
# llvm-mingw ships LLVM's libunwind + compiler-rt instead. Provide shims:
# libgcc_eh -> static libunwind, libgcc -> compiler-rt builtins.
SHIM=$W/gcc-shim
mkdir -p "$SHIM"
cp -f "$W/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a" "$SHIM/libgcc_eh.a"
cp -f "$(ls "$W"/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a | head -1)" "$SHIM/libgcc.a"

echo "Windows toolchain ready in $W (llvm-mingw $LLVM_MINGW_VERSION, SDL2 $SDL2_VERSION)"
