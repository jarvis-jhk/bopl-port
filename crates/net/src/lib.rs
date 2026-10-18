//! # bopl-net
//!
//! Online play for bopl-port: GGRS rollback sessions whose packets travel
//! over Warpzone's encrypted peer channels, and a small host-authoritative
//! lobby/match protocol on the reliable channel. Generic over the simulation
//! ([`NetGame`], any `bytemuck::Pod` input) and the game's settings blob
//! ([`MatchSettings::game`]) and per-player choices ([`SlotRequest::extra`]);
//! it does not depend on bopl-sim. See `docs/net.md`.
//!
//! | Module | What |
//! |---|---|
//! | [`session`] | [`NetSession`]: rooms, lobby, match start/end, `advance()` / `step()` |
//! | [`game`] | [`NetGame`] (the sim as bopl-net sees it), [`Request`], [`fulfil`] |
//! | `matchrun` | GGRS sessions of a match, prediction limit, freeze/resume, spectator stream |
//! | [`input`] | [`NetInput`] and the `Pod` → GGRS input adapter |
//! | [`wire`] | channels, message types, framing |
//! | [`lobby`] | host-side slot/colour/team assignment |
//! | [`transport`] | GGRS socket over Warpzone, host forwarding, link simulation |
//! | [`backend`] | Warpzone client or in-process [`MemHub`] rooms |
//! | [`toy`] | a deterministic toy game, bots, a test driver |

pub mod backend;
pub mod game;
pub mod input;
pub mod lobby;
mod matchrun;
pub mod session;
pub mod toy;
pub mod transport;
pub mod wire;

pub use backend::{MemEndpoint, MemHub};
pub use game::{NetGame, Request, StateCell, Stepped, fulfil};
pub use input::NetInput;
pub use session::{
    DesyncPolicy, GAME_ID, InputStatus, LaunchAction, MachineStats, NetConfig, NetError, NetEvent,
    NetSession, NetStats, NetworkStats, REJECT_CONTENT, REJECT_VERSION,
};
pub use transport::{LinkSim, TransportStats};
pub use warpzone_core;
pub use wire::{
    COLOURS, DesyncReport, DoneReport, GgrsParams, LobbyState, MAX_SLOT_EXTRA, MAX_SLOTS,
    MachineId, MachineView, MatchSettings, MatchStart, Phase, PlayerSlot, SlotRequest, TeamMode,
};
