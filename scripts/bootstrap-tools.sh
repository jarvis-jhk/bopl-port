#!/bin/bash
# Installs every development tool into ./tools (nothing outside the project).
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
mkdir -p tools/lib
if [ ! -x tools/cargo/bin/cargo ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain stable -c rustfmt,clippy
fi
rustup target add x86_64-pc-windows-gnu >/dev/null
if [ ! -x tools/dotnet/dotnet ]; then
  curl -sSfL https://dot.net/v1/dotnet-install.sh -o tools/dotnet-install.sh
  bash tools/dotnet-install.sh --channel 8.0 --install-dir tools/dotnet >/dev/null
fi
command -v ilspycmd >/dev/null || dotnet tool install -g ilspycmd --version 8.2.0.7535 >/dev/null
if [ ! -x tools/venv/bin/python ]; then
  # Ubuntu's python3 lacks ensurepip without python3-venv; use get-pip.
  python3 -m venv --without-pip tools/venv
  curl -sSfL https://bootstrap.pypa.io/get-pip.py -o tools/get-pip.py
  tools/venv/bin/python -I tools/get-pip.py -q
  tools/venv/bin/pip install -q UnityPy Pillow TypeTreeGeneratorAPI
fi
# SDL2: link against the system runtime library (no -dev package needed).
ln -sf /usr/lib/x86_64-linux-gnu/libSDL2-2.0.so.0 tools/lib/libSDL2.so
# Reference decompilation (git-ignored, clean-room reference only).
if [ ! -d ref/decomp/Assembly-CSharp ]; then
  M="$BOPL_GAME_DIR/BoplBattle_Data/Managed"
  ilspycmd -p -o ref/decomp/Assembly-CSharp "$M/Assembly-CSharp.dll" -r "$M" >/dev/null
fi
echo "tools ready"
