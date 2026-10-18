//! The GGRS socket over Warpzone peer channels, host forwarding, and the
//! link conditioner (artificial latency, jitter, loss).

use crate::backend::Backend;
use crate::wire::{self, CH_GGRS, Defragmenter, MachineId, Route};
use std::cell::RefCell;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};
use warpzone_core::client::Event;
use warpzone_core::net::PeerId;

/// Artificial network conditions for outgoing GGRS datagrams. Applied once,
/// by the machine that originates a datagram (the host forwards without
/// adding more), so a guest-to-guest path gets one delay, not two.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkSim {
    /// One-way delay added to every datagram.
    pub delay_ms: u32,
    /// Uniform extra delay in `-jitter..=+jitter` (reorders packets).
    pub jitter_ms: u32,
    /// Probability (0..1) of dropping a datagram.
    pub loss: f32,
}

impl LinkSim {
    /// `BOPL_NET_DELAY_MS`, `BOPL_NET_JITTER_MS`, `BOPL_NET_LOSS` (percent).
    pub fn from_env() -> LinkSim {
        let num = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        LinkSim {
            delay_ms: num("BOPL_NET_DELAY_MS").max(0.0) as u32,
            jitter_ms: num("BOPL_NET_JITTER_MS").max(0.0) as u32,
            loss: (num("BOPL_NET_LOSS") / 100.0).clamp(0.0, 1.0),
        }
    }

    pub fn active(&self) -> bool {
        self.delay_ms > 0 || self.jitter_ms > 0 || self.loss > 0.0
    }
}

/// Traffic counters of this machine's GGRS channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct TransportStats {
    pub sent_datagrams: u64,
    pub sent_bytes: u64,
    pub recv_datagrams: u64,
    pub recv_bytes: u64,
    /// GGRS messages larger than one datagram (sent fragmented).
    pub fragmented: u64,
    /// Dropped by the link conditioner.
    pub sim_dropped: u64,
    /// Host only: datagrams forwarded between guests.
    pub forwarded: u64,
    /// Not sendable (peer not connected yet, unknown machine).
    pub unroutable: u64,
    /// For an older/newer match, or undecodable.
    pub stale: u64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Delayed {
    at: std::cmp::Reverse<(Instant, u64)>,
    peer: PeerId,
    data: Vec<u8>,
}

pub(crate) struct Transport {
    pub backend: Backend,
    pub me: Option<MachineId>,
    pub is_host: bool,
    /// Guest: the host's peer.
    pub host_peer: Option<PeerId>,
    /// Host: who is who.
    pub peer_machine: BTreeMap<PeerId, MachineId>,
    pub machine_peer: BTreeMap<MachineId, PeerId>,
    pub match_tag: u16,
    /// Host: when each machine's last GGRS datagram arrived.
    pub last_recv: BTreeMap<MachineId, Instant>,
    msg_seq: u16,
    defrag: Defragmenter,
    inbound: Vec<(MachineId, ggrs::Message)>,
    /// Non-GGRS events for the session.
    pub pending: VecDeque<Event>,
    pub sim: LinkSim,
    delayed: BinaryHeap<Delayed>,
    seq: u64,
    rng: u64,
    pub stats: TransportStats,
}

impl Transport {
    pub fn new(backend: Backend, sim: LinkSim) -> Transport {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            ^ (std::process::id() as u64) << 32;
        Transport {
            backend,
            me: None,
            is_host: false,
            host_peer: None,
            peer_machine: BTreeMap::new(),
            machine_peer: BTreeMap::new(),
            match_tag: 0,
            last_recv: BTreeMap::new(),
            msg_seq: 0,
            defrag: Defragmenter::default(),
            inbound: Vec::new(),
            pending: VecDeque::new(),
            sim,
            delayed: BinaryHeap::new(),
            seq: 0,
            rng: seed | 1,
            stats: TransportStats::default(),
        }
    }

    pub fn reset_room(&mut self) {
        self.me = None;
        self.is_host = false;
        self.host_peer = None;
        self.peer_machine.clear();
        self.machine_peer.clear();
        self.last_recv.clear();
        self.inbound.clear();
        self.delayed.clear();
    }

    /// Switch to another GGRS session. Messages already decoded for the old
    /// one must not reach the new one (a fresh GGRS endpoint accepts inputs
    /// of any frame before it knows its peer's magic number).
    pub fn set_match_tag(&mut self, tag: u16) {
        self.match_tag = tag;
        self.inbound.clear();
        self.defrag = Defragmenter::default();
    }

    fn rand01(&mut self) -> f32 {
        // xorshift64*: link simulation only, not game state.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Receive, route and forward; deliver delayed datagrams that are due.
    pub fn pump(&mut self) {
        self.flush_delayed();
        self.backend.poll();
        while let Some(e) = self.backend.next_event() {
            match e {
                Event::PeerData {
                    peer,
                    channel: CH_GGRS,
                    data,
                    reliable: false,
                } => self.on_ggrs(peer, &data),
                e => self.pending.push_back(e),
            }
        }
        self.flush_delayed();
    }

    fn on_ggrs(&mut self, peer: PeerId, d: &[u8]) {
        let Some(route) = wire::peek_route(d) else {
            return;
        };
        self.stats.recv_datagrams += 1;
        self.stats.recv_bytes += d.len() as u64;
        if self.is_host {
            // Guests may only speak for themselves.
            if self.peer_machine.get(&peer) != Some(&route.src) {
                return;
            }
            self.last_recv.insert(route.src, Instant::now());
        } else if Some(peer) != self.host_peer {
            return;
        }
        if Some(route.dst) != self.me {
            if self.is_host {
                match self.machine_peer.get(&route.dst) {
                    Some(&to) if self.backend.send(to, CH_GGRS, d, false) => {
                        self.stats.forwarded += 1
                    }
                    _ => self.stats.unroutable += 1,
                }
            }
            return;
        }
        if route.match_tag != self.match_tag {
            self.stats.stale += 1;
            return;
        }
        let Some(payload) = self.defrag.push(d, Instant::now()) else {
            return;
        };
        match wire::decode::<ggrs::Message>(&payload) {
            Some(m) => self.inbound.push((route.src, m)),
            None => self.stats.stale += 1,
        }
    }

    fn peer_for(&self, dst: MachineId) -> Option<PeerId> {
        if self.is_host {
            self.machine_peer.get(&dst).copied()
        } else {
            self.host_peer
        }
    }

    pub fn send_ggrs(&mut self, dst: MachineId, msg: &ggrs::Message) {
        let (Some(me), Some(peer)) = (self.me, self.peer_for(dst)) else {
            self.stats.unroutable += 1;
            return;
        };
        let payload = wire::encode(msg);
        self.msg_seq = self.msg_seq.wrapping_add(1);
        let frames = wire::frame_ggrs(
            Route {
                dst,
                src: me,
                match_tag: self.match_tag,
            },
            self.msg_seq,
            &payload,
        );
        if frames.len() > 1 {
            self.stats.fragmented += 1;
        }
        for f in frames {
            self.submit(peer, f);
        }
    }

    fn submit(&mut self, peer: PeerId, data: Vec<u8>) {
        if !self.sim.active() {
            self.raw_send(peer, &data);
            return;
        }
        if self.sim.loss > 0.0 && self.rand01() < self.sim.loss {
            self.stats.sim_dropped += 1;
            return;
        }
        let j = self.sim.jitter_ms as f32;
        let ms = (self.sim.delay_ms as f32 + (self.rand01() * 2.0 - 1.0) * j).max(0.0);
        let at = Instant::now() + Duration::from_micros((ms * 1000.0) as u64);
        self.seq += 1;
        self.delayed.push(Delayed {
            at: std::cmp::Reverse((at, self.seq)),
            peer,
            data,
        });
    }

    fn flush_delayed(&mut self) {
        let now = Instant::now();
        while self.delayed.peek().is_some_and(|d| d.at.0.0 <= now) {
            let d = self.delayed.pop().unwrap();
            self.raw_send(d.peer, &d.data);
        }
    }

    fn raw_send(&mut self, peer: PeerId, data: &[u8]) {
        if self.backend.send(peer, CH_GGRS, data, false) {
            self.stats.sent_datagrams += 1;
            self.stats.sent_bytes += data.len() as u64;
        } else {
            self.stats.unroutable += 1;
        }
    }

    pub fn take_inbound(&mut self) -> Vec<(MachineId, ggrs::Message)> {
        std::mem::take(&mut self.inbound)
    }
}

/// The GGRS [`ggrs::NonBlockingSocket`] of a match.
pub(crate) struct WzSocket(pub Rc<RefCell<Transport>>);

impl ggrs::NonBlockingSocket<MachineId> for WzSocket {
    fn send_to(&mut self, msg: &ggrs::Message, addr: &MachineId) {
        self.0.borrow_mut().send_ggrs(*addr, msg);
    }

    fn receive_all_messages(&mut self) -> Vec<(MachineId, ggrs::Message)> {
        let mut t = self.0.borrow_mut();
        t.pump();
        t.take_inbound()
    }
}
