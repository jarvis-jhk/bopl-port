#!/bin/bash
# Cross-platform determinism check (M10): the simulation must be bit-identical
# on Linux and Windows, or online matches between the two desync.
#
# 1. bopl-xcheck (scripts/xcheck: the sim + the game's bots, no renderer) runs
#    bot matches for several player counts / team splits / levels / seeds
#    natively and as a Windows exe under Wine (scripts/run-windows.sh) and
#    prints the lookup-table hash and the full World::state_hash every 60
#    ticks: the outputs must be byte-identical.
# 2. --game: the real binaries (bopl-port vs bopl-port.exe) play
#    `--headless --bot-match`; their event logs (deaths, round ends, level
#    loads with their ticks) must be identical; the screenshots are compared
#    and the number of differing pixels reported (presentation uses f32
#    libm sin/cos, whose last bit can differ between glibc and msvcrt: that
#    is cosmetic, not a desync).
#
# Needs an import (BOPL_XCHECK_DATA, default cache/import, else the user data
# dir's import-v1). BOPL_XCHECK_WIN_DATA gives the Windows side its own import
# (e.g. one made by bopl-port.exe: scratch/pkgtest/windows/userdata/import-v1
# after scripts/package-windows.sh --verify), as on two real machines. Builds what is missing (perf-mode recommended):
#   ~/.claude/skills/perf-mode/scripts/perf-mode run -r xcheck -- scripts/check-determinism.sh [--game]
# Output in scratch/xcheck/.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
GAME=0
for a in "$@"; do
    case $a in
        --game) GAME=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
DATA=${BOPL_XCHECK_DATA:-}
for d in "$DATA" cache/import "$BOPL_PORT_DATA_DIR/import-v1"; do
    [ -n "$d" ] && [ -f "$d/index.json" ] && { DATA=$(readlink -f "$d"); break; }
done
[ -f "$DATA/index.json" ] || { echo "no import found (cache/import or \$BOPL_PORT_DATA_DIR/import-v1)" >&2; exit 1; }
WDATA=${BOPL_XCHECK_WIN_DATA:-$DATA}
[ -f "$WDATA/index.json" ] || { echo "no import in $WDATA" >&2; exit 1; }
WDATA=$(readlink -f "$WDATA")
[ "$WDATA" != "$DATA" ] && echo "imports: Linux $DATA, Windows $WDATA"
OUT=scratch/xcheck
rm -rf "$OUT"; mkdir -p "$OUT"

cargo build --release --quiet --manifest-path scripts/xcheck/Cargo.toml --target-dir target/xcheck
scripts/build-windows.sh --xcheck >/dev/null
LX=target/xcheck/release/bopl-xcheck
WX=target/dist/windows/bopl-xcheck.exe

# players teams level seed ticks
RUNS=(
    "12 - 6 1 6000"
    "12 3 27 7 6000"
    "4 - 50 3 6000"
    "8 2 110 5 6000"
    "2 - 125 11 6000"
)
fail=0
for r in "${RUNS[@]}"; do
    read -r p t l s n <<<"$r"
    args=(--players "$p" --level "$l" --seed "$s" --ticks "$n" --every 60)
    [ "$t" != - ] && args+=(--teams "$t")
    tag="p${p}-t${t}-l${l}-s${s}"
    "$LX" --data "$DATA" "${args[@]}" > "$OUT/linux-$tag.txt"
    EXE="$WX" scripts/run-windows.sh --data "$WDATA" "${args[@]}" 2>/dev/null | tr -d '\r' > "$OUT/windows-$tag.txt"
    if [ -s "$OUT/linux-$tag.txt" ] && cmp -s "$OUT/linux-$tag.txt" "$OUT/windows-$tag.txt"; then
        echo "xcheck $tag: identical ($(wc -l < "$OUT/linux-$tag.txt") lines, $(tail -1 "$OUT/linux-$tag.txt"))"
    else
        echo "xcheck $tag: DIFFERENT (diff $OUT/linux-$tag.txt $OUT/windows-$tag.txt)"
        fail=1
    fi
done

if [ $GAME = 1 ]; then
    export CARGO_TARGET_DIR="$BOPL_ROOT/target/package"
    cargo build --release --quiet ${CI:+--locked} -p bopl-port
    LG="$CARGO_TARGET_DIR/release/bopl-port"
    export WGPU_ADAPTER_NAME=${WGPU_ADAPTER_NAME:-llvmpipe} SDL_AUDIODRIVER=dummy RUST_LOG=info
    A=(--headless --data "$DATA" --bot-match --players 12 --seed 1 --level 6 --size 960x540
       --shots 400,1500,2999 --ticks 3000)
    "$LG" "${A[@]}" --screenshot "$OUT/linux-bots.png" > "$OUT/linux-bots.log" 2>&1
    scripts/run-windows.sh "${A[@]}" --screenshot "$OUT/windows-bots.png" > "$OUT/windows-bots.log" 2>&1
    events() { tr -d '\r' < "$1" | grep -E '^\[INFO \] (tick [0-9]+:|next level)' || true; }
    events "$OUT/linux-bots.log" > "$OUT/linux-events.txt"
    events "$OUT/windows-bots.log" > "$OUT/windows-events.txt"
    if [ -s "$OUT/linux-events.txt" ] && cmp -s "$OUT/linux-events.txt" "$OUT/windows-events.txt"; then
        echo "bot match events: identical ($(wc -l < "$OUT/linux-events.txt") events)"
    else
        echo "bot match events: DIFFERENT (diff $OUT/linux-events.txt $OUT/windows-events.txt)"
        fail=1
    fi
    for t in 400 1500 2999; do
        tools/venv/bin/python -I -c '
import sys
from PIL import Image, ImageChops
a, b = (Image.open(p).convert("RGBA") for p in sys.argv[1:3])
d = ImageChops.difference(a, b).tobytes()
n = sum(1 for i in range(0, len(d), 4) if any(d[i:i + 4]))
print(f"bot match shot {sys.argv[3]}: " + ("byte-identical pixels" if n == 0 else f"{n} pixel(s) differ (max {max(d)}/255)"))
' "$OUT/linux-bots-$t.png" "$OUT/windows-bots-$t.png" "$t"
    done
fi
[ $fail = 0 ] && echo "determinism: Linux and Windows agree" || { echo "determinism: MISMATCH" >&2; exit 1; }
