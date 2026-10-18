//! Where peer traffic comes from: a Warpzone [`Client`], or an in-process
//! [`MemHub`] that behaves like a Warpzone room (a star around the host,
//! reliable in-order and unreliable channels, the same events) for tests and
//! for running several sessions in one process.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::time::Duration;
use warpzone_core::client::{Client, Event};
use warpzone_core::net::PeerId;
use warpzone_core::protocol::api::{Audience, Entry, RoomInfo, Visibility};

pub enum Backend {
    Warpzone(Box<Client>),
    Memory(MemEndpoint),
}

impl Backend {
    pub fn poll(&mut self) {
        if let Backend::Warpzone(c) = self {
            c.poll();
        }
    }

    pub fn next_event(&mut self) -> Option<Event> {
        match self {
            Backend::Warpzone(c) => c.next_event(),
            Backend::Memory(m) => m.next_event(),
        }
    }

    pub fn send(&mut self, peer: PeerId, channel: u8, data: &[u8], reliable: bool) -> bool {
        match self {
            Backend::Warpzone(c) => c.send(peer, channel, data, reliable),
            Backend::Memory(m) => m.send(peer, channel, data, reliable),
        }
    }

    pub fn peer_rtt(&self, peer: PeerId) -> Option<Duration> {
        match self {
            Backend::Warpzone(c) => c.peer_rtt(peer),
            Backend::Memory(_) => Some(Duration::ZERO),
        }
    }

    pub fn peer_relayed(&self, peer: PeerId) -> Option<bool> {
        match self {
            Backend::Warpzone(c) => c.peer_relayed(peer),
            Backend::Memory(_) => Some(false),
        }
    }

    pub fn username(&self) -> Option<String> {
        match self {
            Backend::Warpzone(c) => c.account().map(|a| a.username),
            Backend::Memory(m) => Some(m.name()),
        }
    }

    pub fn account_id(&self) -> i64 {
        match self {
            Backend::Warpzone(c) => c.account().map(|a| a.id).unwrap_or(0),
            Backend::Memory(m) => m.hub.0.borrow().nodes[m.id].account,
        }
    }
}

// ------------------------------------------------------------------ memory

struct MemNode {
    name: String,
    account: i64,
    present: bool,
    events: VecDeque<Event>,
}

#[derive(Default)]
struct HubInner {
    nodes: Vec<MemNode>,
    host: Option<usize>,
}

/// An in-process room. The first endpoint is the host; later ones are guests
/// connected only to the host, exactly like a Warpzone room.
#[derive(Clone, Default)]
pub struct MemHub(Rc<RefCell<HubInner>>);

/// One member of a [`MemHub`].
pub struct MemEndpoint {
    hub: MemHub,
    id: usize,
}

fn room_info(host: &str) -> RoomInfo {
    RoomInfo {
        id: "memroom".into(),
        game: crate::session::GAME_ID.into(),
        game_version: "mem".into(),
        name: format!("{host}'s room"),
        host: host.into(),
        players: 0,
        max_players: 12,
        visibility: Visibility::Unlisted,
        entry: Entry::Open,
        audience: Audience::default(),
        ranked: false,
        staged: false,
        lockfile: None,
        preset: None,
        metadata: BTreeMap::new(),
        members: Vec::new(),
        created: 0,
    }
}

// Peer ids as one node sees another: node index + 1.
fn pid(node: usize) -> PeerId {
    node as PeerId + 1
}

impl MemHub {
    pub fn new() -> MemHub {
        MemHub::default()
    }

    /// Add a member: the first one hosts, every later one joins.
    pub fn join(&self, name: &str) -> MemEndpoint {
        let n = self.0.borrow().nodes.len();
        self.join_as(name, n as i64 + 1)
    }

    /// Add a member with a given account id (a rejoining player keeps it).
    pub fn join_as(&self, name: &str, account: i64) -> MemEndpoint {
        let mut h = self.0.borrow_mut();
        let id = h.nodes.len();
        h.nodes.push(MemNode {
            name: name.into(),
            account,
            present: true,
            events: VecDeque::new(),
        });
        match h.host {
            None => {
                h.host = Some(id);
                let room = room_info(name);
                h.nodes[id].events.push_back(Event::RoomHosted {
                    room: room.clone(),
                    code: Some("MEMORY".into()),
                    invite: String::new(),
                });
                h.nodes[id]
                    .events
                    .push_back(Event::RoomJoined { room, host: true });
            }
            Some(host) => {
                let room = room_info(&h.nodes[host].name);
                let host_name = h.nodes[host].name.clone();
                let host_account = h.nodes[host].account;
                let ev = &mut h.nodes[id].events;
                ev.push_back(Event::RoomJoined { room, host: false });
                ev.push_back(Event::PeerJoined {
                    peer: pid(host),
                    account: host_account,
                    username: host_name,
                });
                ev.push_back(Event::PeerConnected {
                    peer: pid(host),
                    relayed: false,
                });
                let hv = &mut h.nodes[host].events;
                hv.push_back(Event::PeerJoined {
                    peer: pid(id),
                    account,
                    username: name.into(),
                });
                hv.push_back(Event::PeerConnected {
                    peer: pid(id),
                    relayed: false,
                });
            }
        }
        MemEndpoint {
            hub: self.clone(),
            id,
        }
    }
}

impl MemEndpoint {
    fn name(&self) -> String {
        self.hub.0.borrow().nodes[self.id].name.clone()
    }

    fn next_event(&mut self) -> Option<Event> {
        self.hub.0.borrow_mut().nodes[self.id].events.pop_front()
    }

    fn send(&mut self, peer: PeerId, channel: u8, data: &[u8], reliable: bool) -> bool {
        let mut h = self.hub.0.borrow_mut();
        let to = peer as usize - 1;
        let host = h.host;
        // Star: only host <-> guest.
        let ok = to < h.nodes.len()
            && h.nodes[to].present
            && h.nodes[self.id].present
            && (Some(to) == host || Some(self.id) == host)
            && to != self.id;
        if ok {
            h.nodes[to].events.push_back(Event::PeerData {
                peer: pid(self.id),
                channel,
                data: data.to_vec(),
                reliable,
            });
        }
        ok
    }

    /// Leave the room (as host: close it).
    pub fn leave(&mut self) {
        let mut h = self.hub.0.borrow_mut();
        if !h.nodes[self.id].present {
            return;
        }
        h.nodes[self.id].present = false;
        if h.host == Some(self.id) {
            for n in h.nodes.iter_mut().filter(|n| n.present) {
                n.events.push_back(Event::RoomClosed {
                    room: "memroom".into(),
                });
                n.present = false;
            }
        } else if let Some(host) = h.host {
            let me = self.id;
            let account = h.nodes[me].account;
            h.nodes[host].events.push_back(Event::PeerLeft {
                peer: pid(me),
                account,
            });
        }
    }
}

impl Drop for MemEndpoint {
    fn drop(&mut self) {
        self.leave();
    }
}
