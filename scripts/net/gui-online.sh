#!/usr/bin/env bash
# Online with the real game on screen (headless): this machine hosts with
# the full game (scripts/dev/m8-online.drive: 4 local players, lobby,
# match, winner screen, back to the lobby, screenshots), two headless
# peers with 4 bot players each join its room: 12 players on 3 machines.
#
#   scripts/net/gui-online.sh [out-dir]      (default scratch/m8)
#   GUI_DRIVE=scripts/dev/m9-online-dots.drive scripts/net/gui-online.sh scratch/m9/dots
#
# Starts scripts/net/dev-server.sh unless WARPZONE_SERVER answers. Wrap in
# perf-mode. Link simulation for every machine: BOPL_NET_DELAY_MS,
# BOPL_NET_JITTER_MS, BOPL_NET_LOSS.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/env.sh"
OUT="${1:-$ROOT/scratch/m8}"
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
mkdir -p "$HOMES/gui-m0" "$HOMES/gui-m1" "$HOMES/gui-m2"
WARPZONE_HOME="$HOMES/gui-m0" RUST_LOG=info timeout 1200 "$BIN" --headless --warpzone-host \
  --drive "${GUI_DRIVE:-$ROOT/scripts/dev/m8-online.drive}" --screenshot "$OUT/online.png" >"$OUT/host.log" 2>&1 &
HOST=$!
PIDS+=("$HOST")
ROOM=""
for _ in $(seq 240); do
  ROOM=$(grep -m1 '^ROOM ' "$OUT/host.log" | awk '{print $2}')
  [ -n "$ROOM" ] && break
  sleep 0.25
done
[ -z "$ROOM" ] && { echo "no room (see $OUT/host.log)"; exit 1; }
echo "room $ROOM"
for i in 1 2; do
  WARPZONE_HOME="$HOMES/gui-m$i" "$BIN" --net-peer --index "$i" --slots 4 \
    --first-slot $((4 * i)) --levels 99 --linger 20 --warpzone-join "$ROOM" >"$OUT/peer$i.log" 2>&1 &
  PIDS+=("$!")
done
wait "$HOST"; RC=$?
sleep 3
grep -h '^RESULT\|match ended\|ERROR' "$OUT"/peer*.log
grep -E 'online match ended|ERROR|desync' "$OUT/host.log"
ls "$OUT"/online-*.png
exit $RC
