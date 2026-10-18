#!/bin/bash
# For every scenario already run (scratch/golden/<name>/cmp.csv): refine the
# alignment with whole-frame matching (refine.py) and compare again with it
# (report-map.txt); summary in scratch/golden/summary-map.txt. Heavy: run under
# perf-mode, after the captures (it renders every tick of every scenario).
#   scripts/dev/golden/refine-all.sh [NAME...]
set -uo pipefail
R="$(cd "$(dirname "$0")/../../.." && pwd)"
source "$R/env.sh"
if [ $# -eq 0 ]; then
  set -- $(for d in "$R"/scratch/golden/*/; do [ -f "$d/cmp.csv" ] && [ -f "$d/capture.mkv" ] && basename "$d"; done)
fi
SUM="$R/scratch/golden/summary-map.txt"
for n in "$@"; do
  D="$R/scratch/golden/$n"
  Y=""; grep -q -- "--y-only" "$R/scripts/dev/golden/scenarios/$n.txt" 2>/dev/null && Y=--y-only
  echo "== $n $(date +%H:%M:%S)" | tee -a "$SUM"
  python -I "$R/scripts/dev/golden/refine.py" "$n" >"$D/refine.log" 2>&1 || { echo "refine failed" | tee -a "$SUM"; continue; }
  "$R/target/release/golden" compare "$D/sim.csv" "$D/track.csv" --map "$D/map.csv" --out "$D/cmp-map.csv" $Y >"$D/report-map.txt" 2>&1
  grep -E "^(fit|player|score)" "$D/report-map.txt" | tee -a "$SUM"
done
