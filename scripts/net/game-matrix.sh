#!/usr/bin/env bash
# Online play with the real game: whole sessions between headless bopl-port
# processes (bots, no GPU) over a local warpzone-server; every machine
# reports the confirmed end frame and checksum of the session (the host's
# first player picks "back to character select" after --levels levels).
#
#   scripts/net/game-matrix.sh [case-filter]
#
# Builds bopl-port (release), starts scripts/net/dev-server.sh unless
# WARPZONE_SERVER answers. Logs: scratch/net/game-matrix/<case>.log, summary
# lines in scratch/net/game-matrix/summary.txt. Wrap in perf-mode: up to 13
# game processes for several minutes. BOPL_NET_DELAY_MS / _JITTER_MS / _LOSS
# apply to cases without their own link flags.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/env.sh"
export BOPL_ROOT="$ROOT"
FILTER="${1:-}"
export WARPZONE_SERVER="${WARPZONE_SERVER:-http://127.0.0.1:${BOPL_WZ_PORT:-3024}}"
OUT="$ROOT/scratch/net/game-matrix"
mkdir -p "$OUT"
SERVER_PID=""
if ! curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null; then
  "$ROOT/scripts/net/dev-server.sh" >"$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 100); do curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null && break; sleep 0.5; done
fi
trap '[ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null' EXIT
(cd "$ROOT" && cargo build -q --release -p bopl-port) || exit 1
BIN="$ROOT/target/release/bopl-port"

declare -a CASES=(
  "2x1-clean|--locals 2x1 --levels 2"
  "3x4-rtt80-loss3|--locals 3x4 --levels 2 --delay 40 --jitter 10 --loss 3"
  "12x1-rtt60-loss2|--locals 12x1 --levels 2 --delay 30 --jitter 10 --loss 2"
  "2-4-6-teams3-code|--locals 2,4,6 --teams 3 --levels 2 --private --delay 20 --jitter 5 --loss 2"
  "1x12-spec|--locals 12 --spectators 1 --levels 2"
  "4x3-relay-spec|--locals 4x3 --spectators 1 --relay --levels 2 --delay 25 --jitter 10 --loss 3"
  "6x2-teams2-rtt100-loss5|--locals 6x2 --teams 2 --levels 2 --delay 50 --jitter 15 --loss 5"
  "4x1-leave-crash|--locals 4x1 --levels 3 --delay 30 --jitter 10 --loss 2 --leave 2@900 --crash 3@1500"
  "2x1-leave-back|--locals 2x1 --levels 5 --delay 30 --loss 2 --leave 1@700"
  "3x1-desync|--locals 3x1 --levels 2 --delay 20 --loss 2 --desync 1@600"
)
FAIL=0
: > "$OUT/summary.txt"
for c in "${CASES[@]}"; do
  name="${c%%|*}"; args="${c#*|}"
  [ -n "$FILTER" ] && [[ "$name" != *$FILTER* ]] && continue
  # shellcheck disable=SC2086
  "$BIN" --net-harness $args --tag "$name" >"$OUT/$name.log" 2>&1 || FAIL=1
  line=$(printf '%-26s %s' "$name" "$(grep '^SUMMARY' "$OUT/$name.log" || echo 'SUMMARY FAIL (no summary)')")
  echo "$line" | tee -a "$OUT/summary.txt"
done
exit $FAIL
