# Online play (`bopl-net`)

bopl-port plays online with GGRS rollback sessions whose packets travel over
[Warpzone](https://github.com/jarvis-jhk/warpzone) peer channels, and
**only** over Warpzone (rooms, codes, room browser, NAT traversal, relay). No
Steam networking. `warpzone-core` is used directly from Rust (git dependency
pinned to `c967f5c1`, the commit checked out in `ext/warpzone`); the game is a
tier-2 Warpzone game with id `bopl-port` (`packaging/warpzone/bopl-port.toml`).
The code is our rounds-port netcode (`rounds-net`) in the later, generic form
of towerfall-port's `tfp-net` (spectators, desync reports and abort policy,
rejoin recognition, slot preview, `NetGame` trait), with Bopl's slot fields
(colour, team, per-player `extra`).

* Each process is a **machine** with 0..12 local player **slots**; a match
  has at most 12 slots in any mix (12 × 1, 1 × 12, 4 × 3, 2 + 4 + 6, ...).
  Machines without slots can **spectate**.
* Fixed 60 Hz step (Bopl's tick, `dt` = `Fix` 1/60); GGRS is told 60 fps.
* The room host is authoritative for the lobby: it assigns machine ids,
  global slots, colours and teams, picks the seed and settings, starts and
  ends matches, decides when a machine has dropped out, and feeds the
  spectators. The host always plays (at least one slot).
* bopl-net does **not** depend on bopl-sim: it is generic over the sim
  (`NetGame`), its per-player input (any `bytemuck::Pod`), the game's
  settings blob (`MatchSettings::game`) and per-player choices
  (`SlotRequest::extra`).

## The game side: `NetGame`

The sim (or a thin adapter in the game crate around it) implements

```rust
pub trait NetGame: Clone + 'static {
    type Input: NetInput;                                   // any bytemuck::Pod
    fn advance(&mut self, inputs: &[(Self::Input, InputStatus)]); // one 1/60 s tick
    fn checksum(&self) -> u128;                             // desync detection
}
```

`Clone` is how rollback saves a frame (keep big immutable data such as level
geometry and the fixed-point lookup tables behind an `Arc`). `inputs[slot]`
is the input of global slot `slot` (`MatchStart::players`). The sim must be
deterministic bit for bit (CLAUDE.md: `Fix` only, no `HashMap` iteration, no
clock, seeded PRNG in the state). `NetSession::step(&mut game)` runs a
frame; `advance()` returns the raw `Request`s (`Save`, `Load`,
`Advance { frame, inputs, resim }`) for a game that wants to interleave its
own bookkeeping. `resim = true` marks a frame simulated again after a
rollback: the presentation layer should not replay its sounds/particles for
it (deduplicate sim events by `(frame, identity)`).

### Input

The input type is bopl-sim's. On the wire an input is its raw bytes
(`input::Wire<I>`): fixed size, no length prefix, zero bytes = no input
(also what a dropped player gets). Keep it small and **state-based** (held
buttons, the sim derives presses by comparing with the previous tick), and
quantise the stick like the original does (one angle byte, 0 = inside the
0.4 dead zone, `1..=255` = direction `2π (b − 1) / 255`): GGRS predicts by
repeating the last input, so stable inputs mean few rollbacks. The original's
sample (net-original.md 4.1) fits in 4 bytes: `angle: u8`, `buttons: u8`
(jump, ability 1–3, start, select), `keys: u8` (W/A/S/D of keyboard
players), one spare byte; `toy::ToyInput` uses exactly that layout.

## Hook points for bopl-port

How Bopl's flow maps onto the generic protocol (match-flow.md 15 decides
what is sim and what is presentation):

| Bopl concept | Where it lives online |
|---|---|
| **Character select** (colour, team, three abilities, ready) | The lobby. Each local box is one `SlotRequest { name, colour, team, extra }`; `extra` carries the ability choices (proposal: `[ab1, ab2, ab3, input_kind]`, ability-list indices; bopl-net only copies the bytes). Call `set_local_slots` whenever a box changes and `set_ready` when the machine's boxes are all ready; every machine sees the roster, and the slot table the next match gets (`LobbyState::preview`, with the host's colour/team resolution), on `LobbyChanged`. |
| Live cursors / "not ready" blinking of remote boxes | Optional `send_app(None, bytes)` → `NetEvent::App` (reliable, through the host; never read by the sim). |
| Number of abilities, level pool, options (the host's values) | The settings blob `MatchSettings::game`, set by the host with `set_settings`; it arrives in `MatchStart::settings.game` on every machine. Version it (first byte) and keep it small; the browser labels `mode` / `map` are separate. |
| Session start | Host `start_match(None)` → `MatchStarted(MatchStart)` everywhere: build the initial sim state from `seed`, `settings.game` and `players` (slot order = player ids 1..N, `colour`, `team`, `extra`). |
| **Mid-game ability select** (loser wheel on the winner screen) | **Sim inputs**: the wheel's choices go into the local players' input (e.g. a state byte "slot k = ability a" that the sim applies idempotently, so GGRS's repeat-last prediction is harmless), and the wheel-open flag that blocks "next level" is an input too. The wheel is drawn from sim state; nothing goes through the lobby. |
| Next level / per-level seed / level choice | Sim (PRNG in the state, derived from `MatchStart::seed`); next/back presses are inputs. |
| "Back to character select" (winner screen) and "fewer than two teams left" | The sim reaches a terminal state; once its frame is confirmed (`frame <= confirmed_frame()`) every machine calls `finish_match(frame, checksum)` and keeps calling `advance` until `MatchEnded`; then everybody is in the lobby again (choices kept). The host can also `end_match()` / `abort_match(reason)`. |
| Pause menu (local overlay) | Presentation: the sim cannot stop for one machine; feed that machine's players neutral input while it is open. |
| Player leaves | Freeze/resume below; the sim sees `InputStatus::Disconnected` for that slot from an agreed frame on. |

A bopl-net "match" is therefore one original *session* (from character
select to the next return to character select), not one round.

## The game side (bopl-port, `crates/game/src/online/`)

Written in M8 on the model of towerfall-port's `online/`.

| File | What |
|---|---|
| `adapter.rs` | `NetWorld` (`NetGame` for the sim session), the settings blob, the per-player `extra`, the starting state from a `MatchStart` |
| `gate.rs` | `EventGate`: sim cues under rollback, each played once |
| `runner.rs` | `OnlineMatch`: one match without graphics (requests, gate, confirmed session end, `finish_match`) |
| `scene.rs` | `OnlineScreen`: Warpzone login, host / join, the lobby (character select with remote boxes) and the match |
| `harness.rs` | `--net-harness` / `--net-peer`: headless multi-process tests |

* **`NetWorld`** wraps `SimMatch` (the `World` plus the level loads its flow
  requests; the converted levels sit in a shared `Rc<LevelCache>`, so a
  clone is the world clone, ~5 µs with 12 players). `advance` maps bopl-net's
  inputs 1:1 onto the sim's player list (global slot = player id − 1).
  `checksum()` = `World::state_hash()` (FNV-1a over all state, 64 bits) in
  the high half, the original's transform checksum and the level in the low
  half.
* **Input** = bopl-sim's `PlayerInput`, 3 bytes: the angle byte (0 = inside
  the dead zone), held buttons (jump, ability 0–2, start, select) and the
  W/A/S/D bits of keyboard + mouse players; state-based, already quantised
  like the original's sample. The mid-game wheel and the winner-screen
  "next level" / "back" run in the sim from these inputs (flow.rs), so they
  are rollback-safe and nothing of them goes through the lobby.
* **Settings blob** (`MatchSettings::game`, `OnlineSettings`):
  `[version 1, number_of_abilities, first_level lo, hi]` (first level =
  build index for tests, `0xffff` = Level1). The host's value of "abilities"
  wins on every machine. `team_mode` is always `Teams(12)`: teams are a free
  per-player choice in Bopl, a box that never touched its team row sends no
  request and the host puts it into the smallest team, so the default is
  free-for-all.
* **`extra`** (`SlotExtra`): `[ability slot 0, slot 1, slot 2, flags]`
  (ability-list indices, bit 0 of flags = keyboard + mouse, which makes the
  sim read the WASD bits). Unknown indices become Random.
* **Seed**: the session seed is `MatchStart::seed` folded to 32 bits; every
  level's seed and the level bag derive from it in the sim.
* **Lobby** = character select (`charsel.rs` online mode): local boxes join
  by device as offline (any mix of keyboard + mouse, keyboard halves,
  controllers); every change re-sends `set_local_slots` (name = Warpzone
  account, `"name k"` for several boxes), the machine is ready when all its
  boxes are. Remote players appear as boxes after the local ones (ready
  face with their name, or the select face while their machine is not
  ready), with the colour and team of the host's preview; local ready boxes
  show the host's colour resolution. The title is "online" + the room code
  (private rooms) or room id (public), a status line under it. The host gets
  the start button once every machine is ready and at least two teams play
  (or one player). Leaving character select (Esc / Back without a box)
  leaves the room.
* **Session end**: "back to character select" on the winner screen (or the
  leave rule below) sets `SimMatch::ended` in the sim; `OnlineMatch` reports
  the first such state's frame and checksum with `finish_match` once its
  inputs are confirmed and simulated (`min(confirmed_frame, frame − 1)`),
  keeps stepping until `MatchEnded`, and everybody is back in the lobby with
  the abilities as edited mid-game (boxes in the select face, ready again).
* **Leave rule** (above) is implemented in the sim (`flow::set_gone`,
  `session.rs`): `NetWorld` marks a slot gone before the first step whose
  input is `Disconnected`; its bopl is inert for the rest of the round (not
  counted by `teams_left`, cannot win, no loser circle for teammates kept
  "alive" by it, its wheel closes), it does not spawn from the next level
  on, its score stays (greyed circle on the winner screen), and fewer than
  two connected teams request "back to character select". Test:
  `crates/content/tests/flow.rs` (`players_who_left_...`).
* **Presentation under rollback**: the screen shows the predicted state
  (the match scene mirrors `NetWorld` after every step). Cues go through
  `EventGate`: each `(tick, kind and subject without positions, n-th)`
  plays once (first seen), so re-simulated frames are silent and corrected
  frames play only what is new; identities of confirmed ticks are dropped.
  Sim ticks equal net frames (the world starts at tick 0).
* **Pacing**: one net step per 60 Hz tick; while `frames_ahead() > 0` one
  tick in ten skips its step (≈ 10 % slower). Headless online runs pace
  themselves to the wall clock (`HeadlessOptions::realtime`).
* **Launch**: `--warpzone-host` hosts a public room at once;
  `--warpzone-join ROOM|CODE|invite-link`, `WARPZONE_JOIN`, `WARPZONE_HOST=1`
  as in the descriptor. Main menu → Online: host game (public, listed),
  host private (unlisted, code), join code (typed on the keyboard; up to 10
  characters, so a public room id works too), room list, name. The Warpzone
  identity lives in `WARPZONE_HOME`; without an account one is registered
  from the chosen name or the OS user name (suffixed if taken).
* **Room browser** (`online/browser.rs`, main menu → online → room list):
  its own Warpzone session lists the public rooms (`list_rooms(true)`,
  game `bopl-port`) on login and every 4 s; each row shows the room name
  (`"<host>'s <preset>"`), the slots of the next match (room metadata
  `players`) / max players, the mode unless the name says it, and "(other
  version)" for rooms of another `game_version` (refused locally). Six
  rows per page (left/right), refresh, back. Choosing a room drops the
  browser's session and joins by room id.
* **Player name** (main menu → online → name, `Settings::player_name` in
  `settings.json`): the slot name everybody sees (`"name k"` for several
  local boxes); empty = the Warpzone account's username. Also registers a
  new identity under that name.
* **Version check**: rooms only accept `bopl-port <version> net<PROTOCOL>`
  (Warpzone's `game_version` and the `Hello`). Both sides must also have
  imported the same game build (the sim reads level and ability data from
  the import): the `Hello` carries `NetConfig::content_id`, which bopl-port
  sets to `online::content_id` = FNV-1a 64 over the import's sim inputs
  (`abilities.json`, `player.json`, `globals.json`, `animations.json`,
  `prefab_index.json`, every `levels/*.json` and `prefabs/*.json`, names and
  bytes, sorted; ≈ 18 MB, computed when the online screen opens;
  `BOPL_CONTENT_ID` overrides it for tests). A mismatch gets
  `Reject {"content mismatch: host <id>, guest <id>"}` (a version mismatch
  `"version mismatch: host runs ..."`); the guest leaves on `Reject`
  (the host kicks it 1.5 s later if it does not) and the main menu's
  error line explains: "different bopl battle data than the host (host
  8858cc, you deadbe): every machine needs the same bopl battle build".
  `scripts/net/m9-online-ui.sh` tests both (screenshots
  `scratch/m9/online-ui-*.png`, `online-reject-*.png`).
* **Busy dots**: a remote player's open wheel (sim state, so every machine
  knows it) shows as the loser circle's three bouncing dots instead of the
  wheel, like the original (`MatchUi::remote`).

## Topology

Warpzone rooms are a **star**: a guest is connected only to the host
(directly after NAT punching, or through the Warpzone relay). GGRS is a full
mesh, so every GGRS datagram carries `dst`/`src` machine ids and the host
forwards guest-to-guest datagrams unchanged (it checks that `src` is the
sending guest). Consequences: guest-to-guest latency is two hops; the host's
upload carries `O(machines²)` streams (measured: 4 machines at 60 Hz, about
760 forwarded datagrams/s; fine for 12 on a home connection, but a weak host
uplink hurts everyone).

## Channels and framing

All encodings are bincode 1 with varint integers and a 64 KiB limit.

| Channel | Mode | Content |
|---|---|---|
| 1 `CH_GGRS` | unreliable | `kind:u8 dst:u8 src:u8 tag:u16le` + one bincode `ggrs::Message`; kind 1 = whole, kind 2 = fragment (`msg_id:u16le idx:u8 count:u8` + part; reassembled, incomplete ones expire after 2 s). `tag` = `match_id << 4 \| epoch`: datagrams of another match or epoch are dropped. |
| 2 `CH_LOBBY` | reliable, ordered | `LobbyMsg`, split into chunks of `more:u8` + ≤ 1099 bytes |

## Lobby and match protocol

| Message | Direction | Meaning |
|---|---|---|
| `Hello {protocol, game_version, slots, spectate, content}` | guest → host | first message (`content` since protocol 2); a mismatching version or game-data id gets `Reject {reason}`, the guest leaves, the host kicks it after 1.5 s if not |
| `SetSlots`, `SetReady`, `SetSpectate` | guest → host | local players (`name`, `colour`, team request, `extra`), ready flag, watch matches |
| `Welcome {machine}` | host → guest | our machine id (host = 0, then 1, 2, ... in join order, never reused in a room) |
| `State(LobbyState)` | host → all | phase, roster (machines, their slot requests, ready/waiting/spectate, `rejoined_from`), settings, and `preview`: the slot table the next match would get |
| `Start(MatchStart)` | host → players, spectators | `match_id`, `seed`, settings, the slot table, GGRS parameters, spectators |
| `Done {match_id, frame, checksum}` | guest → host | the sim ended at confirmed `frame` |
| `End {match_id, reports, desyncs}` | host → all | every playing machine reported (or dropped): drop the session; everyone compares the checksums |
| `Freeze` / `Frozen` / `Resume` / `Dropped` | both | machines dropping out mid-match, see below |
| `Desync {match_id, report}` | guest → host | GGRS found a checksum mismatch with another machine |
| `Abort {match_id, reason}` | host → all | match aborted (host action, desync policy) |
| `Spec {match_id, first, frames}` | host → spectators | final inputs of frames `first..`, see Spectators |
| `App {from, to, data}` | any → any via host | game data outside the sim (chat, menu cursors) |

**Settings** (`MatchSettings`, host-chosen, part of `MatchStart`):
`team_mode` (`Ffa` or `Teams(n)`, n = 2..=12), `mode` and `map` (labels for
the room browser), and `game`: the game's **settings blob**, opaque bytes
bopl-port defines (see "Hook points" above). Room metadata for the browser:
`players` (slots in the next match), `mode` (default "free-for-all" /
"n teams"), `map` — the descriptor's `display` keys. `list_rooms(has_space)`
returns them in `Event::RoomList`.

**Slot assignment** (`lobby::assign`, unit-tested): machines by id, each
machine's slots in its order, until 12 (the rest sit out;
`accepted_slots` says how many fit).
*Colours* (12, unique per player, as in the original): a request is honoured
if that colour is free, else the slot gets the next free colour after it
(wrapping), like the original's character select moving a clashing box to
the next free colour to the right. 12 slots always get 12 distinct colours.
*Teams*: free-for-all: team = slot. `Teams(n)`: a valid request is honoured,
otherwise the smallest team (lowest index on ties); the host can override
with `set_team`. *Extra*: `SlotRequest::extra` (≤ 64 bytes) is copied into
`PlayerSlot::extra` unchanged.

Everybody builds the same GGRS P2P session from `MatchStart`: players =
slots, own slots `Local`, others `Remote(machine)`. Machines that join while
a match runs are marked `waiting` and play from the next `start_match`.

**Ending a match**: when the sim decides the match is over at a *confirmed*
frame, the game calls `finish_match(frame, checksum)` and keeps calling
`advance` (others may still need our inputs) until `MatchEnded` arrives.
Leaving the GGRS session earlier would make the others see us drop.

## Leaving, dropping out, rejoining

**The rule.** A machine that leaves (or crashes, or goes silent for 2 s)
during a match: from an agreed frame `F` on, its slots get the zero input
with `InputStatus::Disconnected` on every machine. What the sim does with
that is bopl-sim's decision, made deterministically from the statuses (they
are the same on every machine). The contract proposed for bopl-sim (the toy
implements it and the tests check it):

1. For the rest of the current round its bopls are **inert**: no input, so
   they stand (or fall) where they are and can be killed as usual. They do
   not block the round end: a round whose remaining *connected* players are
   all on one team ends as it normally would.
2. At the **round end** they are **removed**: they do not spawn in later
   levels, their score stays on the winner screen (greyed), they cannot win.
3. If fewer than two teams with connected players remain, the sim goes back
   to character select (the original does this too, match-flow.md 13): the
   game calls `finish_match` and everybody returns to the lobby.

The original instead sends everybody back to character select as soon as
anyone leaves ("instant back", net-original.md 3.3); bopl-sim can choose
that (rule 3 for any drop) without any change here.

**How the net layer gets to `F`** (freeze/resume). GGRS 0.13 cannot drop a
peer reliably with more than two machines: every machine disconnects it at
the last frame *it* received, the views converge on the minimum by gossip,
and the resulting rollback can reach behind inputs GGRS already discarded or
behind its save window — it panics (reproduced in rounds-net with 4 machines
and 2 % loss). bopl-net therefore never lets GGRS disconnect anyone (GGRS
timeout 600 s) and does it itself:

1. The host notices a machine is gone: Warpzone `PeerLeft`, or no GGRS
   datagram from it for `disconnect_timeout_ms` (2 s; 10 s before the
   session is synchronised).
2. It sends `Freeze {epoch, gone}` to the others (and `Dropped` to the gone
   machine if it can still be reached). Every machine stops advancing and
   answers `Frozen {frame}`: the newest frame `r` whose inputs were all
   confirmed when it last simulated and whose successor state `r + 1` is
   saved (GGRS keeps `rollback_window + 1 = 25` states; we predict at most
   `max_prediction = 8` frames, so the frame is always still there).
3. When all answered (3 s timeout: the silent ones are dropped too, new
   epoch), the host sends `Resume {frame: F = min(r) + 1, gone}`.
4. Every machine loads its saved state of `F` and continues in a new GGRS
   session (new `tag`) in which the gone machines' slots are dummy local
   players of the host that always send zero. Frame numbers keep counting.
   The first `input_delay` frames of a new session have zero inputs for
   everybody (GGRS fills its input delay with defaults) — the same on every
   machine.

The game sees `MatchFrozen {gone}` (show "a player left, resyncing…"), then
`PlayersDropped {machines, slots, from_frame}`, and handles the `Load` that
comes with the next `advance`. A freeze costs about one round trip plus a
GGRS re-synchronisation (a few hundred ms).

**Rejoin.** A machine that left can join the room again (same code / room
id). It gets a new machine id (ids are never reused); the host recognises
the Warpzone account (`MachineView::rejoined_from`,
`MachineJoined { rejoined_from }`) so the lobby can show "Ann is back" and
the game can restore her colour and abilities. A running match is not re-entered
(there is no mid-match state transfer): the machine waits and plays from the
next match. **The host leaving ends the room** (Warpzone has no host
migration): guests get `RoomLeft` / `MatchAborted`.

## Spectators

A machine with no slots that called `set_spectate(true)` watches every match
that starts while it is in the room (`MatchStart::spectators`,
`NetSession::is_spectator`). It does not join the GGRS session. The host
records the inputs of every frame it simulates and streams frames that are
**final** to the spectators on the reliable channel (`Spec`, batches of ≥ 4
frames, ≤ 256 per message): frames at least `max_prediction + input_delay +
4` (14) frames behind the host's confirmed frame, so that a freeze — which
restarts at most that far before the host's confirmed frame — never rewrites
a frame a spectator already has. The spectator simulates them without
rollback (`advance` returns only `Advance` requests, inputs `Confirmed` or
`Disconnected`), catching up 2–4 frames per step when far behind. At the end
the host flushes everything confirmed before `End`, and the spectator emits
`MatchEnded` once it simulated the last frame. Cost: 12 × 4 bytes + 3 per
frame, ~3 KB/s per spectator; spectators are ~250 ms behind plus their
link. Spectators leaving do not affect the match.

## Desync detection and reporting

* GGRS desync detection is on (`desync_interval` = 10 frames): every save
  carries the game's checksum, peers exchange the checksums of confirmed
  frames, a mismatch gives `NetEvent::Desync {frame, local, remote, machine}`
  locally.
* Each machine reports the first desync per peer per match to the host
  (`LobbyMsg::Desync`); the host emits `DesyncReported(report)` and collects
  them (≤ 64 per match) into `End` → `MatchEnded { desyncs }` on everyone.
* `finish_match` checksums are compared by everyone (`MatchEnded { agreed }`).
* Policy (`NetConfig::desync_policy`): `Report` (default: keep playing,
  show a warning, save a replay/log) or `Abort` (the host aborts the match
  for everybody on the first report: `MatchAborted { reason: "desync ..." }`).

## Statistics

`stats()` → `NetStats`: current/confirmed frame, frames ahead, rollbacks
(Load requests), re-simulated frames, deepest rollback, prediction stalls,
desyncs, resumes, spectator buffer, transport counters (datagrams, bytes,
fragments, forwarded, dropped by the link simulator) and per machine: GGRS
ping / kbps / frames behind, Warpzone RTT of our link to it, `relayed`,
`via_host`. `last_match_stats()` keeps them after the match.

## API for the game crate

```rust
use bopl_net::{LaunchAction, MatchSettings, NetConfig, NetEvent, NetSession, SlotRequest, TeamMode};
use bopl_net::warpzone_core::client::{Event, HostOptions};

let mut cfg = NetConfig::new(env!("CARGO_PKG_VERSION"));  // rooms only accept the same version
cfg.auto_register = None;                     // or Some(name) after asking the player
let mut net: NetSession<NetWorld> = NetSession::new(cfg)?;   // NetWorld: NetGame<Input = bopl_sim::Input>
match net.handle_launch(&std::env::args().collect::<Vec<_>>()) {
    LaunchAction::Joining { .. } => show_connecting(),
    LaunchAction::HostScreen => open_host_screen(), // then net.host(HostOptions { preset: Some("ffa".into()), .. })
    LaunchAction::None => {}
}
// character select: one request per local box, again whenever it changes
net.set_local_slots(vec![SlotRequest { name: "Ann".into(), colour: 6, team: None, extra: vec![3, 7, 12] }]);
net.set_ready(true);
// host: net.set_settings(MatchSettings { team_mode: TeamMode::Ffa, mode, map, game: blob })?; net.start_match(None)
// browse: net.list_rooms(true) -> NetEvent::Warpzone(Event::RoomList(..)); net.join(id, None, None)
// code:   net.join_code("K7QM3X")

// every frame
net.poll();
while let Some(e) = net.next_event() {
    match e {
        NetEvent::MatchStarted(ms) => world = NetWorld::new(&ms),   // seed, settings.game, slot table
        NetEvent::LobbyChanged => redraw_lobby(net.lobby()),       // roster + preview
        NetEvent::Warpzone(e) => match *e { Event::RoomHosted { code, invite, .. } => .., _ => {} },
        NetEvent::MatchEnded { .. } | NetEvent::MatchAborted { .. } => back_to_character_select(),
        _ => {}
    }
}
// every 1/60 s tick (run ~10 % slower while net.frames_ahead() > 0)
for &slot in net.local_slots() { net.add_local_input(slot, read_pad(slot))?; }
let stepped = net.step(&mut world);          // or handle net.advance() requests yourself
if world.session_over() && world.frame() <= net.confirmed_frame() {
    net.finish_match(world.frame(), world.checksum())?;
}
```

Other calls: `register`, `host`, `join`, `join_code`, `list_rooms`,
`answer_knock`, `kick`, `set_room_closed`, `leave`, `set_ready`,
`set_spectate`, `set_team`, `end_match`, `abort_match`, `send_app`,
`lobby()`, `me()`, `is_host()`, `is_spectator()`, `current_match()`,
`is_running()`, `current_frame()`, `confirmed_frame()`, `stats()`,
`last_match_stats()`, `with_warpzone(|c| ..)` (the raw
`warpzone_core::client::Client`). Everything is single-threaded: call it
from the game thread (the Warpzone client runs its own HTTP threads).
`GAME_ID` = `"bopl-port"`; the Warpzone identity lives in `WARPZONE_HOME`
(dev: `cache/warpzone`, set by env.sh).

`NetSession::memory(MemHub::join(name), cfg)` gives the same API over an
in-process room (no server) — used by the tests and by `netsim memory`, and
usable for debugging several sessions in one process.

## Link simulation

Outgoing GGRS datagrams can be delayed, jittered and dropped by the machine
that originates them (the host's forwarding adds nothing, so a
guest-to-guest path gets one delay): `NetConfig::link_sim` or
`BOPL_NET_DELAY_MS` (one way), `BOPL_NET_JITTER_MS` (uniform ±),
`BOPL_NET_LOSS` (percent). `BOPL_NET_FORCE_RELAY=1` routes every Warpzone
connection through the server relay. The lobby channel is not affected.

## Testing

```sh
source env.sh
cargo test -p bopl-net                         # unit tests + in-process matches (memory rooms)
scripts/net/dev-server.sh                      # local warpzone-server, http://127.0.0.1:3024, udp 3504
#   BOPL_WZ_PORT=3025 BOPL_WZ_UDP_PORT=3505 scripts/net/dev-server.sh   if taken
scripts/net/dev-server.sh --check              # warpzone-server check-games packaging/warpzone
export WARPZONE_SERVER=http://127.0.0.1:3024
cargo run -p bopl-net --example netsim -- run --locals 4x3 --frames 3600
cargo run -p bopl-net --example netsim -- run --locals 12x1 --delay 50 --jitter 20 --loss 5
cargo run -p bopl-net --example netsim -- run --locals 2,4,6 --teams 3
cargo run -p bopl-net --example netsim -- run --locals 4x3 --relay --spectators 1
cargo run -p bopl-net --example netsim -- run --locals 4x1 --leave 2@900 --crash 3@1500
cargo run -p bopl-net --example netsim -- run --locals 3x1 --desync 1@600 [--abort-on-desync]
cargo run -p bopl-net --example netsim -- memory --locals 12x1 --delay 40 --loss 5   # one process, no server
target/release/bopl-port --net-harness --locals 3x4 --levels 2 --delay 40 --loss 3   # the real game
perf-mode run -r "game matrix" -- scripts/net/game-matrix.sh [filter]
~/.claude/skills/perf-mode/scripts/perf-mode run -r "online on screen" -- scripts/net/gui-online.sh
~/.claude/skills/perf-mode/scripts/perf-mode run -r "net matrix" -- scripts/net/netsim-matrix.sh [frames] [filter]
```

`dev-server.sh` builds `warpzone-server` and `wz` from `ext/warpzone` into
`scratch/net/wz-target/` (ext/ is only read), writes its config and database
to `scratch/net/wz-dev/` (registration and login limits raised: tests create
a dozen identities from one IP) and serves `packaging/warpzone/` as its games
directory (validated with `check-games` on every start). Ports 3024/3504
avoid Warpzone's defaults (2998/3478), rounds-port's (2998/3478 and
3998/3479 for its online test) and towerfall-port's (3012/3490), which run
on the same laptop.

`netsim` plays `bopl_net::toy`: a 320×240 arena with up to 12 dots
("bopls") that walk, jump and use three abilities (pellet, dash, high jump,
blink) with cooldowns, no friendly fire in teams, screen wrap, and rounds
every 600 ticks; every round after the first opens with a 90-tick
ability-select phase in which the ability buttons cycle the abilities
(mid-game selection as sim input). The initial abilities come from each
slot's `extra`, the round length from the settings blob. Integer physics, a
PRNG in the state, bots that hold an input for 4–40 frames. `run` uses one
OS process per machine, each with its own `WARPZONE_HOME` under
`scratch/net/homes/`. Machine 0 hosts a public room; odd machines join by
code, even ones through `--warpzone-join <room>` (the launcher path); the
last `--spectators N` machines watch. At the target frame every player
reports its checksum through `finish_match`; the orchestrator prints
`SUMMARY PASS` when all remaining machines (spectators included) report the
same one and no desync was seen. `--leave M@F` leaves gracefully,
`--crash M@F` exits without a word, `--desync M@F` corrupts machine M's
state at frame F (PASS = detected and reported, or with `--abort-on-desync`
aborted everywhere), `--teams N` plays N teams.

### Results

2026-10-17, `scripts/net/netsim-matrix.sh 3600`: 3600 frames (60 s) per case,
one OS process per machine against the local dev server, one laptop. Delays
are one way per originating machine, so a guest-to-guest round trip is about
twice the value plus the host hop. Rollbacks are summed over machines.

| Case | Machines × slots | Link | Result | Rollbacks | Max depth |
|---|---|---|---|---|---|
| 4x3-clean | 4 × 3 | none | PASS | 2 | 1 |
| 12x1-clean | 12 × 1 | none | PASS | 0 | 0 |
| mixed-2-4-6 | 2 + 4 + 6 | none | PASS | 16 | 6 |
| mixed-2-4-6-teams3 | 2 + 4 + 6, 3 teams | 20 ms ± 5, 2 % loss | PASS | 409 | 6 |
| 4x3-rtt50-loss5 | 4 × 3 | 25 ms ± 10, 5 % loss | PASS | 1320 | 8 |
| 4x3-rtt150-loss5 | 4 × 3 | 75 ms ± 25, 5 % loss | PASS | 4603 | 8 |
| 12x1-rtt100-loss5 | 12 × 1 | 50 ms ± 20, 5 % loss | PASS | 14295 | 8 |
| 4x3-relay-spec | 4 × 3 + 1 spectator | forced relay (all links relayed) | PASS (spectator agrees) | 0 | 0 |
| 6x2-relay-rtt100-loss5 | 6 × 2 | forced relay + 50 ms ± 15, 5 % loss | PASS | 6842 | 6 |
| 4x1-leave-crash | 4 × 1 | 30 ms ± 10, 2 % loss; m2 leaves @900, m3 crashes @1500 | PASS (2 left, 2 resumes) | 218 | 8 |
| 6x2-rtt100-leave-crash-spec | 6 × 2 + 1 spectator | 50 ms ± 15, 5 % loss; leave @800, crash @1600 | PASS (4 + spectator left, 2 resumes) | 3639 | 8 |
| 3x1-desync | 3 × 1 | 20 ms ± 5, 2 % loss; m1 corrupts its state @600 | PASS: detected (1179 GGRS events, 12 pair reports to the host), final checksums disagree | 63 | 2 |
| 3x1-desync-abort | 3 × 1 | none; m2 corrupts @600, abort policy | PASS: detected, match aborted on all 3 | 6 | 6 |

No desync in any case without injection; the deepest rollback is the
prediction limit (8). Every match spans six toy rounds, so five mid-game
ability-select phases (choices as inputs) are part of every compared
checksum. `cargo test -p bopl-net` (13 unit tests, 12 in-process matches,
~18 s) additionally runs: 2 + 1 + 3, 2 + 4 + 6 with 200-tick rounds (two
ability-select phases before the target), 4 × 3 with 40 ms / 5 % loss,
12 × 1, 1 × 12, a graceful leave, two crashes plus a leave in a 5-machine
match, two spectators, a spectator through a drop, desync detection +
reporting, the abort policy, and rejoin recognition; each start also checks
unique colours and that `extra` arrives unchanged.

### Game matrix (bopl-port, M8)

The real game: `scripts/net/game-matrix.sh` runs `bopl-port --net-harness`
per case: one headless process per machine (no GPU), bots on every slot
(`bots.rs`: they fight, losers use the wheel, they press "next level"), real
levels from Level1 on; after `--levels N` levels the host's first player
picks "back to character select" on the winner screen, which ends the
session in the sim, and every machine reports the confirmed end frame and
checksum (`finish_match`). `SUMMARY PASS` = all remaining machines report,
equal checksums, the host saw agreement, no desync (desync case: the
injected desync was detected). 2026-10-17, local dev server, one laptop;
abilities still stubs:

| Case | Machines × slots | Link | Result | Frames | Rollbacks | Max depth |
|---|---|---|---|---|---|---|
| 2x1-clean | 2 × 1 | none | PASS | 1888 | 0 | 0 |
| 3x4-rtt80-loss3 | 3 × 4 | 40 ms ± 10, 3 % loss | PASS | 9777 | 11199 | 4 |
| 12x1-rtt60-loss2 | 12 × 1 | 30 ms ± 10, 2 % loss | PASS | 5029 | 20887 | 4 |
| 2-4-6-teams3-code | 2 + 4 + 6, 3 teams, joined by code | 20 ms ± 5, 2 % loss | PASS | 8321 | 2045 | 2 |
| 1x12-spec | 1 × 12 + 1 spectator | none | PASS (spectator agrees) | 4333 | 0 | 0 |
| 4x3-relay-spec | 4 × 3 + 1 spectator | forced relay, 25 ms ± 10, 3 % loss | PASS | 5694 | 2542 | 3 |
| 6x2-teams2-rtt100-loss5 | 6 × 2, 6v6 | 50 ms ± 15, 5 % loss | PASS | 3726 | 11619 | 6 |
| 4x1-leave-crash | 4 × 1 | 30 ms ± 10, 2 % loss; leave @900, crash @1500 | PASS (2 remain, 2 resumes each; the session goes on) | 4380 | 765 | 6 |
| 2x1-leave-back | 2 × 1 | 30 ms, 2 % loss; leave @700 | PASS (one team left: back to the lobby at frame 700) | 700 | 148 | 2 |
| 3x1-desync | 3 × 1 | 20 ms, 2 % loss; m1 corrupts its RNG @600 | PASS: detected (745 GGRS desync events); the states converge again at the next level load (the RNG is re-seeded) | 3674 | 52 | 2 |

Also: `BOPL_NET_DELAY_MS=40 BOPL_NET_JITTER_MS=10 BOPL_NET_LOSS=3
bopl-port --net-harness --locals 3x4 --levels 1` (link from the environment)
PASS, 2972 rollbacks; `scripts/net/gui-online.sh` with
`BOPL_NET_DELAY_MS=30 BOPL_NET_JITTER_MS=10 BOPL_NET_LOSS=2`: the full game
hosting 4 local players (keyboard + mouse and 3 controllers, driven) and two
peers with 4 bots each, lobby → match (7611 frames) → "back to character
select" → lobby, checksums agree, `scratch/m8/online-*.png`.

## Open issues

* **No guest-to-guest links in Warpzone** (rooms are a star): GGRS traffic
  between guests is forwarded by the host. A mesh option in `warpzone-core`
  would halve guest-to-guest latency and remove the host's `O(n²)`
  forwarding load.
* **No host migration**: the host leaving closes the room and ends the
  match.
* **GGRS 0.13 drop handling** is worked around by freeze/resume; a GGRS
  release that drops peers at an agreed frame would make it unnecessary.
* **No mid-match rejoin**: needs a serialisable sim state (state transfer
  from the host); `NetGame` only requires `Clone`. Rejoiners play from the
  next session.
* **Spectators are host-fed**: they cost the host ~3 KB/s each and see the
  match ~250 ms late; the host cannot spectate (it must play: it coordinates
  drops, plays dropped slots' dummies and feeds spectators). Spectators must
  be in the room when the match starts.
* `Client::force_relay` is a no-op until the client has its UDP side (after
  the first `LoggedIn`); bopl-net re-applies it on `LoggedIn` and before
  host/join.
* Players who join a running match wait for the next one (no mid-match
  join, see above); remote cursors (`send_app`) are not shown.
* The game-data id hashes the converted files, so two machines on the same
  Bopl Battle build but different import format revisions (different
  bopl-port versions) differ too — the version check already refuses those.
* The descriptor's `[[data]]` hands the Bopl install to the game as
  `WARPZONE_DATA_BOPL` and runs `bopl-port --import --game DIR` until
  `import-v1/index.json` exists; the game binary must accept both. The
  release asset names in `[install]`/`[launch]` are placeholders until
  packaging (M11).
