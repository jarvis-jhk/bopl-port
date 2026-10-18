#!/usr/bin/env bash
# Run the bopl-net multi-process test matrix against a local warpzone-server.
#
#   scripts/net/netsim-matrix.sh [frames] [case-filter...]   # default 3600 frames (60 s) per case
#
# Filters are substrings of case names; a case runs if it matches any.
#
# Starts scripts/net/dev-server.sh in the background unless WARPZONE_SERVER
# already answers. Logs: scratch/net/matrix/<case>.log, summary on stdout.
# Wrap in perf-mode: it keeps up to 13 game processes busy for minutes.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/env.sh"
FRAMES="${1:-3600}"
shift || true
FILTERS=("$@")
export WARPZONE_SERVER="${WARPZONE_SERVER:-http://127.0.0.1:${BOPL_WZ_PORT:-3024}}"
OUT="$ROOT/scratch/net/matrix"
mkdir -p "$OUT"
SERVER_PID=""
if ! curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null; then
  "$ROOT/scripts/net/dev-server.sh" >"$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 100); do curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null && break; sleep 0.5; done
fi
trap '[ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null' EXIT
BIN="$ROOT/target/debug/examples/netsim"
# Other crates of the workspace may be mid-edit: fall back to the last build.
if ! (cd "$ROOT" && cargo build -q -p bopl-net --example netsim); then
  [ -x "$BIN" ] || exit 1
  echo "warning: build failed, using the existing $BIN" >&2
fi

declare -a CASES=(
  "4x3-clean|--locals 4x3"
  "12x1-clean|--locals 12x1"
  "mixed-2-4-6|--locals 2,4,6"
  "mixed-2-4-6-teams3|--locals 2,4,6 --teams 3 --delay 20 --jitter 5 --loss 2"
  "4x3-rtt50-loss5|--locals 4x3 --delay 25 --jitter 10 --loss 5"
  "4x3-rtt150-loss5|--locals 4x3 --delay 75 --jitter 25 --loss 5"
  "12x1-rtt100-loss5|--locals 12x1 --delay 50 --jitter 20 --loss 5"
  "4x3-relay-spec|--locals 4x3 --relay --spectators 1"
  "6x2-relay-rtt100-loss5|--locals 6x2 --relay --delay 50 --jitter 15 --loss 5"
  "4x1-leave-crash|--locals 4x1 --delay 30 --jitter 10 --loss 2 --leave 2@900 --crash 3@1500"
  "6x2-rtt100-leave-crash-spec|--locals 6x2 --spectators 1 --delay 50 --jitter 15 --loss 5 --leave 2@800 --crash 4@1600"
  "3x1-desync|--locals 3x1 --delay 20 --jitter 5 --loss 2 --desync 1@600"
  "3x1-desync-abort|--locals 3x1 --desync 2@600 --abort-on-desync"
)
FAIL=0
for c in "${CASES[@]}"; do
  name="${c%%|*}"; args="${c#*|}"
  if [ ${#FILTERS[@]} -gt 0 ]; then
    hit=0
    for f in "${FILTERS[@]}"; do [[ "$name" == *"$f"* ]] && hit=1; done
    [ $hit = 1 ] || continue
  fi
  # shellcheck disable=SC2086
  "$BIN" run $args --frames "$FRAMES" --tag "$name" >"$OUT/$name.log" 2>&1 || FAIL=1
  printf '%-30s %s\n' "$name" "$(grep '^SUMMARY' "$OUT/$name.log" || echo 'SUMMARY FAIL (no summary)')"
done
exit $FAIL
