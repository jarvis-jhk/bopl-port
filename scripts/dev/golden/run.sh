#!/bin/bash
# Golden comparison of one scenario with the ORIGINAL (docs/reference-capture.md,
# "Golden comparisons"): scenario -> .rep -> our sim trace; the original plays the
# .rep headless on :66 (Steam mode) while ffmpeg records; the capture is tracked
# (bopl blobs by player colour, camera from the static platforms) and compared.
#
#   scripts/dev/golden/run.sh SCENARIO.txt [--no-capture] [--capture-only] [--retrack]
#                             [--extra SECONDS]
#
# Outputs in scratch/golden/<name>/: <name>.rep, sim.csv(+.events), capture.mkv
# (never committed), track.csv, cmp.csv, report.txt. --no-capture re-tracks and
# re-compares an existing capture (e.g. after a sim fix: only sim + compare rerun).
set -euo pipefail
R="$(cd "$(dirname "$0")/../../.." && pwd)"
source "$R/env.sh"
SC=$1; shift
NAME=$(basename "$SC" .txt)
OUT="$R/scratch/golden/$NAME"
CAPTURE=1; EXTRA=6; RETRACK=0; CAPTURE_ONLY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --no-capture) CAPTURE=0 ;;
    --retrack) RETRACK=1 ;;
    --capture-only) CAPTURE_ONLY=1 ;;
    --extra) EXTRA=$2; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
mkdir -p "$OUT" "$R/scratch/golden/ref"
G="$R/target/release/golden"
[ -x "$G" ] || cargo build --release -p bopl-content --bin golden
"$G" gen "$SC" "$OUT/$NAME.rep"
"$G" sim "$OUT/$NAME.rep" "$OUT/sim.csv"
LEVEL=$(awk '$1=="level"{print $2}' "$SC"); LEVEL=${LEVEL:-6}
COLORS=$(awk '$1=="player"{c=""; for(i=2;i<=NF;i++) if($i ~ /^color=/){split($i,a,"="); c=a[2]} if(c=="") c=n+1; n++; printf "%s%s", (n>1?",":""), c}' "$SC")
REF0="$R/scratch/golden/ref/L$LEVEL-c0.png"; REF1="$R/scratch/golden/ref/L$LEVEL-c10.png"
if [ ! -f "$REF1" ]; then
  "$R/target/release/bopl-port" --headless --level "$LEVEL" --camera-x 0 --time 4 --screenshot "$REF0" >/dev/null 2>&1
  "$R/target/release/bopl-port" --headless --level "$LEVEL" --camera-x 10 --time 4 --screenshot "$REF1" >/dev/null 2>&1
fi
if [ $CAPTURE = 1 ] || [ ! -f "$OUT/capture.mkv" ]; then
  N=$("$G" frames "$OUT/$NAME.rep")
  # Start-up takes 6..25 s (more on a busy machine): generous slack.
  "$R/scripts/dev/golden/capture.sh" "$OUT/$NAME.rep" "$OUT/capture.mkv" $(( N / 60 + 24 + EXTRA ))
  RETRACK=1
  rm -f "$OUT/track.csv"
fi
[ $CAPTURE_ONLY = 1 ] && exit 0
if [ $RETRACK = 1 ] || [ ! -f "$OUT/track.csv" ]; then
  nice -n 10 "$G" track "$OUT/capture.mkv" "$OUT/track.csv" --ref "$REF0" --ref2 "$REF1" --colors "$COLORS"
fi
# Scenarios whose comment says "compare --y-only" (no static platforms).
Y=""; grep -q -- "--y-only" "$SC" && Y=--y-only
"$G" compare "$OUT/sim.csv" "$OUT/track.csv" --out "$OUT/cmp.csv" $Y | tee "$OUT/report.txt"
grep -v shake "$OUT/sim.csv.events" | head -20 | sed 's/^/sim event: /' | tee -a "$OUT/report.txt"
