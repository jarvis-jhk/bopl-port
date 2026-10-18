//! Everything bopl-net puts on Warpzone channels.
//!
//! * Channel [`CH_GGRS`], unreliable: GGRS datagrams, framed with
//!   destination/source machine and match tag so the room host can forward
//!   them (Warpzone rooms are a star: guests only talk to the host).
//! * Channel [`CH_LOBBY`], reliable: [`LobbyMsg`] between a guest and the host,
//!   split into chunks of at most [`MAX_DATAGRAM`] bytes.
//!
//! Encoding: bincode 1 with varint integers and a 64 KiB size limit, on both
//! channels. Field order is the format: append new fields only at the end of
//! messages and bump [`PROTOCOL`] when changing anything.

use bincode::Options;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub use warpzone_core::protocol::MAX_DATAGRAM;

/// Protocol version; machines with a different one are rejected in `Hello`.
pub const PROTOCOL: u16 = 2;
/// Unreliable GGRS traffic.
pub const CH_GGRS: u8 = 1;
/// Reliable lobby/match control.
pub const CH_LOBBY: u8 = 2;

/// Most player slots in one match.
pub const MAX_SLOTS: usize = 12;
/// Machine 0 is always the room host.
pub const HOST_MACHINE: MachineId = MachineId(0);

/// A machine (one game process, one Warpzone member) in the room. Assigned by
/// the host: 0 for itself, then 1, 2, ... in join order, never reused within
/// one room. This is the GGRS address type.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct MachineId(pub u8);

impl std::fmt::Display for MachineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "m{}", self.0)
    }
}

fn opts() -> impl Options {
    bincode::DefaultOptions::new().with_limit(64 * 1024)
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    opts().serialize(v).expect("serialisable")
}

pub fn decode<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Option<T> {
    opts().deserialize(b).ok()
}

// ------------------------------------------------------------------ lobby

/// Number of distinct player colours (Bopl has 12, unique per player).
pub const COLOURS: u8 = 12;
/// Size limit of [`SlotRequest::extra`].
pub const MAX_SLOT_EXTRA: usize = 64;

/// What a machine asks for one of its local players (its character-select
/// box).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SlotRequest {
    /// Display name (the game decides; trimmed to 16 chars by the host).
    pub name: String,
    /// Preferred colour index (`0..COLOURS`); the host resolves clashes.
    pub colour: u8,
    /// Preferred team in team modes.
    pub team: Option<u8>,
    /// Game-defined per-player choices shown to everyone and copied into
    /// the match (bopl-port: the three chosen abilities, input kind, ...),
    /// at most [`MAX_SLOT_EXTRA`] bytes; opaque to bopl-net.
    pub extra: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TeamMode {
    /// Every player on their own team (team = slot index).
    #[default]
    Ffa,
    /// `n` teams (2..=12; the original offers up to 4).
    Teams(u8),
}

/// Host-chosen settings that every machine needs to build an identical match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MatchSettings {
    pub team_mode: TeamMode,
    /// Shown in the room browser (`mode`, `map` metadata), e.g.
    /// "free-for-all", "Snow"; not used for the match itself.
    pub mode: String,
    pub map: String,
    /// The game's settings blob, opaque here (bopl-port: level pool, ability
    /// pool, options; see docs/net.md). Every machine builds the match from
    /// the same bytes.
    pub game: Vec<u8>,
}

/// GGRS tuning; part of [`MatchStart`] so that every machine uses the host's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GgrsParams {
    pub input_delay: u8,
    /// How many frames we run ahead of the last confirmed frame before
    /// waiting (bopl-net enforces this itself).
    pub max_prediction: u8,
    /// GGRS's own prediction window = how many saved states it keeps, i.e.
    /// the deepest possible rollback. Larger than `max_prediction` so that
    /// the extra rollback after a machine drops (see docs/net.md) fits.
    pub rollback_window: u8,
    /// Desync detection every n frames (0 = off).
    pub desync_interval: u16,
    /// Host: a playing machine silent this long is dropped from the match.
    pub disconnect_timeout_ms: u32,
    pub disconnect_notify_ms: u32,
    /// Frame rate GGRS assumes for time sync (60: Bopl's tick is 1/60 s).
    pub fps: u16,
}

impl Default for GgrsParams {
    fn default() -> Self {
        GgrsParams {
            input_delay: 2,
            max_prediction: 8,
            rollback_window: 24,
            desync_interval: 10,
            disconnect_timeout_ms: 2000,
            disconnect_notify_ms: 500,
            fps: 60,
        }
    }
}

/// One player slot of a started match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSlot {
    /// Global slot index = GGRS player handle = index into the inputs.
    pub slot: u8,
    pub machine: MachineId,
    /// Index into that machine's own slot list.
    pub local_index: u8,
    pub name: String,
    /// Unique among the slots (while there are no more slots than colours).
    pub colour: u8,
    pub team: u8,
    /// The slot's [`SlotRequest::extra`].
    pub extra: Vec<u8>,
}

/// Everything a machine needs to build the match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchStart {
    pub match_id: u32,
    pub seed: u64,
    pub settings: MatchSettings,
    pub players: Vec<PlayerSlot>,
    pub ggrs: GgrsParams,
    /// Machines watching (no slots): they get the confirmed inputs from the
    /// host and simulate along, a little behind.
    pub spectators: Vec<MachineId>,
}

impl MatchStart {
    /// Machines that play in this match, ascending.
    pub fn machines(&self) -> Vec<MachineId> {
        let mut v: Vec<MachineId> = self.players.iter().map(|p| p.machine).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn slots_of(&self, m: MachineId) -> Vec<usize> {
        self.players
            .iter()
            .filter(|p| p.machine == m)
            .map(|p| p.slot as usize)
            .collect()
    }
}

/// A machine as the lobby shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineView {
    pub machine: MachineId,
    /// Warpzone account id (0 if unknown).
    pub account: i64,
    pub name: String,
    pub ready: bool,
    /// Joined while a match was running; plays from the next match.
    pub waiting: bool,
    /// No slots, wants to watch matches.
    pub spectate: bool,
    /// The same account was machine `m` earlier in this room (rejoin).
    pub rejoined_from: Option<MachineId>,
    pub slots: Vec<SlotRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Phase {
    #[default]
    Lobby,
    InMatch {
        match_id: u32,
    },
}

/// The host's lobby, broadcast to every guest whenever it changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LobbyState {
    pub phase: Phase,
    pub machines: Vec<MachineView>,
    pub settings: MatchSettings,
    /// The slot table the next match would get (preview of the host's
    /// colour/team resolution), for the lobby screen.
    pub preview: Vec<PlayerSlot>,
}

/// One machine's end-of-match report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneReport {
    pub machine: MachineId,
    pub frame: i32,
    pub checksum: u128,
}

/// A desync seen by `reporter` against `with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesyncReport {
    pub reporter: MachineId,
    pub with: MachineId,
    pub frame: i32,
    pub local: u128,
    pub remote: u128,
}

/// The final inputs of one frame, for spectators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecFrame {
    /// Bit `slot`: that slot is disconnected (its input is zero).
    pub gone: u16,
    /// Every slot's input bytes, concatenated in slot order.
    pub inputs: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyMsg {
    // guest -> host
    Hello {
        protocol: u16,
        game_version: String,
        slots: Vec<SlotRequest>,
        spectate: bool,
        /// Identity of the game data the sim reads (`NetConfig::content_id`;
        /// protocol 2).
        content: String,
    },
    SetSlots(Vec<SlotRequest>),
    SetReady(bool),
    SetSpectate(bool),
    /// The sim finished the match at confirmed `frame` with this checksum.
    Done {
        match_id: u32,
        frame: i32,
        checksum: u128,
    },
    /// Answer to `Freeze`: newest frame whose successor state is saved and
    /// final here.
    Frozen {
        match_id: u32,
        epoch: u8,
        frame: i32,
    },
    /// GGRS detected a desync with another machine.
    Desync {
        match_id: u32,
        report: DesyncReport,
    },
    // host -> guest
    Welcome {
        machine: MachineId,
    },
    Reject {
        reason: String,
    },
    State(LobbyState),
    Start(MatchStart),
    /// Machines dropped out: stop advancing and answer `Frozen`.
    Freeze {
        match_id: u32,
        epoch: u8,
        gone: Vec<MachineId>,
    },
    /// You were dropped from the match (timed out).
    Dropped {
        match_id: u32,
    },
    /// Continue from sim frame `frame` without `gone` (their slots idle).
    Resume {
        match_id: u32,
        epoch: u8,
        frame: i32,
        gone: Vec<MachineId>,
    },
    /// Every playing machine reported `Done` (or left): drop the session.
    End {
        match_id: u32,
        reports: Vec<DoneReport>,
        desyncs: Vec<DesyncReport>,
    },
    /// The host aborted the match (desync policy, host action).
    Abort {
        match_id: u32,
        reason: String,
    },
    /// Spectators: the final inputs of frames `first`, `first + 1`, ...
    Spec {
        match_id: u32,
        first: i32,
        frames: Vec<SpecFrame>,
    },
    // any -> any, forwarded by the host
    App {
        from: MachineId,
        to: Option<MachineId>,
        data: Vec<u8>,
    },
}

/// Split a reliable message into chunks: `[more:u8] bytes`.
pub fn chunk(msg: &[u8]) -> Vec<Vec<u8>> {
    let n = MAX_DATAGRAM - 1;
    let parts: Vec<&[u8]> = if msg.is_empty() {
        vec![&[][..]]
    } else {
        msg.chunks(n).collect()
    };
    let last = parts.len() - 1;
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut v = Vec::with_capacity(p.len() + 1);
            v.push(u8::from(i != last));
            v.extend_from_slice(p);
            v
        })
        .collect()
}

/// Reassembles reliable chunks (they arrive in order per peer).
#[derive(Default)]
pub struct Dechunker {
    buf: Vec<u8>,
}

impl Dechunker {
    pub fn push(&mut self, chunk: &[u8]) -> Option<Vec<u8>> {
        let (&more, rest) = chunk.split_first()?;
        self.buf.extend_from_slice(rest);
        if self.buf.len() > 256 * 1024 {
            self.buf.clear();
            return None;
        }
        (more == 0).then(|| std::mem::take(&mut self.buf))
    }
}

// ------------------------------------------------------------------ GGRS frames

const K_WHOLE: u8 = 1;
const K_FRAG: u8 = 2;
/// kind, dst, src, match tag (u16 le).
pub const GGRS_HEADER: usize = 5;
const FRAG_HEADER: usize = GGRS_HEADER + 4;

/// Header of a datagram on [`CH_GGRS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub dst: MachineId,
    pub src: MachineId,
    pub match_tag: u16,
}

pub fn peek_route(d: &[u8]) -> Option<Route> {
    if d.len() < GGRS_HEADER || !(d[0] == K_WHOLE || d[0] == K_FRAG) {
        return None;
    }
    Some(Route {
        dst: MachineId(d[1]),
        src: MachineId(d[2]),
        match_tag: u16::from_le_bytes([d[3], d[4]]),
    })
}

/// Frame one GGRS message into datagrams of at most [`MAX_DATAGRAM`] bytes.
pub fn frame_ggrs(route: Route, msg_id: u16, payload: &[u8]) -> Vec<Vec<u8>> {
    let head = |kind: u8| {
        let t = route.match_tag.to_le_bytes();
        vec![kind, route.dst.0, route.src.0, t[0], t[1]]
    };
    if payload.len() + GGRS_HEADER <= MAX_DATAGRAM {
        let mut v = head(K_WHOLE);
        v.extend_from_slice(payload);
        return vec![v];
    }
    let n = MAX_DATAGRAM - FRAG_HEADER;
    let parts: Vec<&[u8]> = payload.chunks(n).collect();
    let count = parts.len().min(255) as u8;
    parts
        .iter()
        .take(255)
        .enumerate()
        .map(|(i, p)| {
            let mut v = head(K_FRAG);
            v.extend_from_slice(&msg_id.to_le_bytes());
            v.push(i as u8);
            v.push(count);
            v.extend_from_slice(p);
            v
        })
        .collect()
}

struct Partial {
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
    born: Instant,
}

/// Reassembles fragmented GGRS messages (unreliable: incomplete ones expire).
#[derive(Default)]
pub struct Defragmenter {
    partial: BTreeMap<(MachineId, u16), Partial>,
}

impl Defragmenter {
    /// Returns the GGRS payload once complete.
    pub fn push(&mut self, d: &[u8], now: Instant) -> Option<Vec<u8>> {
        let route = peek_route(d)?;
        if d[0] == K_WHOLE {
            return Some(d[GGRS_HEADER..].to_vec());
        }
        if d.len() < FRAG_HEADER {
            return None;
        }
        let id = u16::from_le_bytes([d[5], d[6]]);
        let (idx, count) = (d[7] as usize, d[8] as usize);
        if count == 0 || idx >= count {
            return None;
        }
        self.partial
            .retain(|_, p| now.duration_since(p.born) < Duration::from_secs(2));
        let p = self
            .partial
            .entry((route.src, id))
            .or_insert_with(|| Partial {
                parts: vec![None; count],
                have: 0,
                born: now,
            });
        if p.parts.len() != count {
            return None;
        }
        if p.parts[idx].is_none() {
            p.parts[idx] = Some(d[FRAG_HEADER..].to_vec());
            p.have += 1;
        }
        if p.have < count {
            return None;
        }
        let p = self.partial.remove(&(route.src, id))?;
        Some(p.parts.into_iter().flatten().flatten().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_roundtrip() {
        for len in [0usize, 1, MAX_DATAGRAM - 1, MAX_DATAGRAM, 5000] {
            let msg: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let mut d = Dechunker::default();
            let mut out = None;
            for c in chunk(&msg) {
                assert!(c.len() <= MAX_DATAGRAM);
                assert!(out.is_none());
                out = d.push(&c);
            }
            assert_eq!(out.unwrap(), msg);
        }
    }

    #[test]
    fn ggrs_frames_roundtrip_out_of_order() {
        let route = Route {
            dst: MachineId(3),
            src: MachineId(1),
            match_tag: 77,
        };
        let now = Instant::now();
        for len in [0usize, 10, MAX_DATAGRAM - GGRS_HEADER, MAX_DATAGRAM, 4000] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
            let mut frames = frame_ggrs(route, 9, &payload);
            frames.reverse();
            let mut d = Defragmenter::default();
            let mut out = None;
            for f in &frames {
                assert!(f.len() <= MAX_DATAGRAM);
                assert_eq!(peek_route(f), Some(route));
                out = out.or(d.push(f, now));
            }
            assert_eq!(out.unwrap(), payload, "len {len}");
        }
    }

    #[test]
    fn lobby_msg_roundtrip() {
        let m = LobbyMsg::Hello {
            protocol: PROTOCOL,
            game_version: "0.1.0".into(),
            slots: vec![SlotRequest {
                name: "A".into(),
                colour: 2,
                team: Some(1),
                extra: vec![1, 2, 3],
            }],
            spectate: false,
            content: "c0ffee".into(),
        };
        assert_eq!(decode::<LobbyMsg>(&encode(&m)), Some(m));
        assert_eq!(decode::<LobbyMsg>(&[0xff, 0xff, 0xff]), None);
        let s = LobbyMsg::Spec {
            match_id: 3,
            first: 100,
            frames: vec![
                SpecFrame {
                    gone: 0b100,
                    inputs: vec![1; 48],
                };
                8
            ],
        };
        let b = encode(&s);
        assert!(b.len() < 450, "8 frames of 12 players: {} bytes", b.len());
        assert_eq!(decode::<LobbyMsg>(&b), Some(s));
    }
}
