#!/bin/bash
# Builds dist/bopl-port-<version>-linux-x86_64.tar.gz (and dist/SHA256SUMS).
#
# Layout (one top-level folder, as the Warpzone descriptor expects:
# exec = "bopl-port-*/play.sh"):
#   bopl-port-<version>/{bopl-port, play.sh, README.txt, LICENSE-*,
#                        THIRD_PARTY_NOTICES.md, licenses/, [lib/libSDL2-2.0.so.0]}
#
# Usage: scripts/package-linux.sh [--bundle-sdl] [--no-build] [--verify]
#   --bundle-sdl  also ship the build host's libSDL2-2.0.so.0 in lib/ (play.sh
#                 puts it first on LD_LIBRARY_PATH). Off by default: SDL2 is on
#                 every desktop distro and on the Steam Deck; a bundled copy
#                 built against the build host's X11/Wayland/audio libraries is
#                 often worse than the system's own. The binary's NEEDED entry
#                 is the soname, so it runs against any SDL2 2.x.
#   --no-build    reuse the last build (target/package/release/bopl-port)
#   --verify      unpack the archive into scratch/pkgtest/linux and smoke-test
#                 it: --version, --list-devices and, when the game is installed
#                 ($BOPL_GAME_DIR), `--import --game` into a data dir inside
#                 scratch/ plus a headless level screenshot
#
# Build on the oldest glibc you want to support (CI: ubuntu-22.04 = glibc
# 2.35, which covers the Steam Deck). Release builds go to target/package so
# they never wait for, or invalidate, the development builds in target/.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
source scripts/package-common.sh

BUNDLE_SDL=0 BUILD=1 VERIFY=0
for a in "$@"; do
    case $a in
        --bundle-sdl) BUNDLE_SDL=1 ;;
        --no-build) BUILD=0 ;;
        --verify) VERIFY=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

VERSION=$(pkg_version)
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
NAME="bopl-port-$VERSION"
OUT="$DIST/$NAME-linux-x86_64.tar.gz"
STAGE="$DIST/.stage-linux"
EPOCH=$(pkg_epoch)
TD="${BOPL_PKG_TARGET_DIR:-$BOPL_ROOT/target/package}"

if [ $BUILD = 1 ]; then
    cargo build --release ${CI:+--locked} --target-dir "$TD" -p bopl-port
fi
BIN="$TD/release/bopl-port"
[ -x "$BIN" ] || { echo "$BIN missing" >&2; exit 1; }

# The binary must not need a newer glibc than the oldest system we support.
MAX_GLIBC="${BOPL_MAX_GLIBC:-2.35}"
if command -v readelf >/dev/null; then
    # Version requirements flagged WEAK (Rust std's optional symbols) do not
    # stop the loader on an older glibc, so they don't count.
    need=$(readelf -V "$BIN" | grep -E 'Name: GLIBC_[0-9.]+ +Flags: none' | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
    if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
        echo "$BIN needs glibc $need > $MAX_GLIBC: build on an older system (BOPL_MAX_GLIBC overrides)" >&2
        exit 1
    fi
    echo "glibc needed: ${need:-none} (limit $MAX_GLIBC)"
fi

pkg_clean_stage "$STAGE"
PKG="$STAGE/$NAME"
mkdir -p "$PKG"
install -m 755 "$BIN" "$PKG/bopl-port"
if command -v strip >/dev/null; then
    strip --strip-all "$PKG/bopl-port"
else
    echo "warning: strip not found, shipping the unstripped binary" >&2
fi
install -m 755 packaging/linux/play.sh "$PKG/play.sh"
install -m 644 packaging/linux/README.txt "$PKG/README.txt"
pkg_stage_docs "$PKG" x86_64-unknown-linux-gnu

if [ $BUNDLE_SDL = 1 ]; then
    sdl=$(ldd "$BIN" | awk '/libSDL2-2\.0\.so\.0/ {print $3}')
    [ -f "$sdl" ] || { echo "libSDL2-2.0.so.0 not found" >&2; exit 1; }
    mkdir -p "$PKG/lib"
    install -m 755 "$(readlink -f "$sdl")" "$PKG/lib/libSDL2-2.0.so.0"
    install -m 644 packaging/licenses/SDL2-LICENSE.txt "$PKG/licenses/SDL2-LICENSE.txt"
fi

mkdir -p "$DIST"
chmod -R go-w "$STAGE"
find "$STAGE" -exec touch -h -d "@$EPOCH" {} +
(cd "$STAGE" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$EPOCH" -cf - "$NAME") \
    | gzip -9n > "$OUT.part"
mv "$OUT.part" "$OUT"
rm -rf "$STAGE"
scripts/check-no-game-data.sh --archives "$OUT"

if [ $VERIFY = 1 ]; then
    V="$BOPL_ROOT/scratch/pkgtest/linux"
    rm -rf "$V"; mkdir -p "$V"
    tar -xzf "$OUT" -C "$V"
    P="$V/$NAME"
    test -x "$P/play.sh" && test -x "$P/bopl-port"
    # The archive's own data dir, inside scratch/ (never ~/.local/share).
    export BOPL_PORT_DATA_DIR="$V/userdata"
    "$P/play.sh" --version
    SDL_AUDIODRIVER=dummy SDL_VIDEODRIVER=dummy "$P/play.sh" --list-devices | sed -n 1,12p
    if pkg_have_game; then
        "$P/play.sh" --import --game "$BOPL_GAME_DIR"
        test -f "$BOPL_PORT_DATA_DIR/import-v1/index.json"
        SDL_AUDIODRIVER=dummy "$P/play.sh" --headless --level 6 --size 1280x720 --screenshot "$V/level6.png"
        test -s "$V/level6.png" && echo "packaged Linux build: import + headless level screenshot ok ($V/level6.png)"
    else
        echo "Bopl Battle not installed: skipped the import and screenshot test"
    fi
fi

scripts/package-checksums.sh "$DIST" >/dev/null
echo "$OUT ($(human "$OUT"))"
