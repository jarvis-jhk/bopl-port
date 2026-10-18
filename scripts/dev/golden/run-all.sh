#!/bin/bash
# Runs scripts/dev/golden/run.sh for every scenario (or the given ones):
# all captures first, then tracking + comparison; the reports are collected
# in scratch/golden/summary.txt. Run under perf-mode; each scenario takes its
# length in real time plus about a minute. --no-capture: re-track/compare only.
#   scripts/dev/golden/run-all.sh [--no-capture] [SCENARIO.txt...]
set -uo pipefail
R="$(cd "$(dirname "$0")/../../.." && pwd)"
OPTS=()
if [ "${1:-}" = --no-capture ]; then OPTS+=(--no-capture); shift; fi
if [ $# -eq 0 ]; then set -- "$R"/scripts/dev/golden/scenarios/*.txt; fi
SUM="$R/scratch/golden/summary.txt"
# Captures first (the original needs a quiet machine), then tracking.
if [ ${#OPTS[@]} -eq 0 ]; then
  for sc in "$@"; do
    n=$(basename "$sc" .txt)
    echo "capture $n $(date +%H:%M:%S)"
    "$R/scripts/dev/golden/run.sh" "$sc" --capture-only >"$R/scratch/golden/$n.capture.log" 2>&1
  done
fi
for sc in "$@"; do
  n=$(basename "$sc" .txt)
  echo "== $n $(date +%H:%M:%S)" | tee -a "$SUM"
  "$R/scripts/dev/golden/run.sh" "$sc" --no-capture >"$R/scratch/golden/$n.log" 2>&1
  grep -E "^(fit|player|score|sim event)" "$R/scratch/golden/$n/report.txt" 2>/dev/null | tee -a "$SUM"
done
