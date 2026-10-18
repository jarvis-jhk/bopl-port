#!/usr/bin/env bash
# M9 online UI test (headless): a --net-peer hosts a public room, the full
# game browses the room list, enters a player name, joins and shows the
# lobby (scripts/dev/m9-online.drive); a second run with other game data
# (BOPL_CONTENT_ID) is refused with the reason on the menu
# (scripts/dev/m9-online-reject.drive). Screenshots in OUT (scratch/m9).
#
#   scripts/net/m9-online-ui.sh [out-dir]
#
# Starts scripts/net/dev-server.sh unless WARPZONE_SERVER answers. Wrap in
# perf-mode.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/env.sh"
OUT="${1:-$ROOT/scratch/m9}"
mkdir -p "$OUT"
export WARPZONE_SERVER="${WARPZONE_SERVER:-http://127.0.0.1:${BOPL_WZ_PORT:-3024}}"
SERVER_PID=""
if ! curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null; then
  "$ROOT/scripts/net/dev-server.sh" >"$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 100); do curl -sf "$WARPZONE_SERVER/api/v1/info" >/dev/null && break; sleep 0.5; done
fi
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done; [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null; }
trap cleanup EXIT
(cd "$ROOT" && cargo build -q --release -p bopl-port) || exit 1
BIN="$ROOT/target/release/bopl-port"
HOMES="$ROOT/scratch/net/homes"
mkdir -p "$HOMES/ui-host" "$HOMES/ui-game" "$HOMES/ui-other"
WARPZONE_HOME="$HOMES/ui-host" "$BIN" --net-peer --host --index 0 --slots 2 --expect 9 \
  --levels 1 >"$OUT/ui-host.log" 2>&1 &
PIDS+=("$!")
for _ in $(seq 240); do grep -q '^ROOM ' "$OUT/ui-host.log" && break; sleep 0.25; done
grep -m1 '^ROOM ' "$OUT/ui-host.log" || { echo "no room (see $OUT/ui-host.log)"; exit 1; }
RC=0
WARPZONE_HOME="$HOMES/ui-game" RUST_LOG=info timeout 300 "$BIN" --headless --realtime \
  --drive "$ROOT/scripts/dev/m9-online.drive" --screenshot "$OUT/online-ui.png" \
  >"$OUT/online-ui.log" 2>&1 || RC=1
WARPZONE_HOME="$HOMES/ui-other" BOPL_CONTENT_ID=deadbeefdeadbeef RUST_LOG=info timeout 300 "$BIN" --headless --realtime \
  --drive "$ROOT/scripts/dev/m9-online-reject.drive" --screenshot "$OUT/online-reject.png" \
  >"$OUT/online-reject.log" 2>&1 || RC=1
grep -h 'rejected\|game data id\|my slots\|ERROR' "$OUT"/online-ui.log "$OUT"/online-reject.log
ls "$OUT"/online-ui-*.png "$OUT"/online-reject-*.png
exit $RC
