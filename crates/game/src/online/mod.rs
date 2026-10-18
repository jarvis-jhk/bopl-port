//! Online play (M8): bopl-net's rollback sessions over Warpzone wired to
//! the simulation and the screens. See docs/net.md, "The game side".
//!
//! | Module | What |
//! |---|---|
//! | [`adapter`] | `NetWorld` (`bopl_net::NetGame` for the sim session), the settings blob, the per-player `extra`, the match from a `MatchStart` |
//! | [`gate`] | `EventGate`: sim cues under rollback (each played once) |
//! | [`runner`] | `OnlineMatch`: one match without graphics (requests, gate, match end) |
//! | [`scene`] | `OnlineScreen`: host / join, the lobby (character select with remote boxes) and the match |
//! | [`harness`] | `--net-harness` / `--net-peer`: headless multi-process tests |

pub mod adapter;
pub mod browser;
pub mod gate;
pub mod harness;
pub mod runner;
pub mod scene;

use bopl_net::NetSession;

use adapter::NetWorld;

/// Machines only play together with the same game and net protocol.
pub fn net_version() -> String {
    format!(
        "bopl-port {} net{}",
        env!("CARGO_PKG_VERSION"),
        bopl_net::wire::PROTOCOL
    )
}

/// Identity of the converted Bopl Battle data the simulation reads: a
/// 64-bit FNV-1a hash over the level, prefab, ability, player, globals and
/// animation files of the import (byte-identical for the same game build
/// and import format on every machine), as 16 hex digits. Machines with a
/// different one are rejected by the host (docs/net.md, "Version check").
pub fn content_id(data: &bopl_import::GameData) -> String {
    let dir = &data.dir;
    let mut files: Vec<std::path::PathBuf> = [
        "abilities.json",
        "player.json",
        "globals.json",
        "animations.json",
        "prefab_index.json",
    ]
    .iter()
    .map(|f| dir.join(f))
    .collect();
    for sub in ["levels", "prefabs"] {
        let mut v: Vec<_> = std::fs::read_dir(dir.join(sub))
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        v.sort();
        files.extend(v);
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    };
    for f in &files {
        let name = f
            .strip_prefix(dir)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        eat(name.as_bytes());
        eat(&std::fs::read(f).unwrap_or_default());
    }
    format!("{h:016x}")
}

/// The content id this machine announces: `BOPL_CONTENT_ID` (tests of the
/// mismatch message), else [`content_id`].
pub fn local_content_id(data: &bopl_import::GameData) -> String {
    std::env::var("BOPL_CONTENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| content_id(data))
}

/// A player-facing message for a refused join (`NetEvent::Rejected`).
pub fn reject_message(reason: &str, mine: &str) -> String {
    if reason.starts_with(bopl_net::REJECT_CONTENT) {
        let host = reason
            .split("host ")
            .nth(1)
            .and_then(|r| r.split(',').next())
            .unwrap_or("?");
        format!(
            "different bopl battle data than the host (host {}, you {}): every machine needs the same bopl battle build",
            &host[..host.len().min(6)],
            &mine[..mine.len().min(6)]
        )
    } else if reason.starts_with(bopl_net::REJECT_VERSION) {
        let host = reason.split("host runs ").nth(1).unwrap_or("?");
        format!(
            "different bopl-port version than the host ({host}; you run {}): everybody needs the same version",
            net_version()
        )
    } else {
        format!("refused: {reason}")
    }
}

/// A join code (`K7QM3X`, typed by players) rather than a room id
/// (`v7thncejf4`, from the launcher or an invite).
pub fn is_code(s: &str) -> bool {
    (4..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Join by code, room id or invite link (`--warpzone-join VALUE`).
pub fn join(net: &mut NetSession<NetWorld>, value: &str) {
    let v = value.trim();
    if is_code(v) {
        net.join_code(&v.to_ascii_uppercase());
    } else {
        net.handle_launch(&["--warpzone-join", v]);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_and_rooms() {
        assert!(super::is_code("K7QM3X"));
        assert!(super::is_code("k7qm3x"));
        assert!(!super::is_code("v7thncejf4"));
        assert!(!super::is_code("warpzone://join/ab12cd"));
    }

    #[test]
    fn reject_messages() {
        let m = super::reject_message(
            "content mismatch: host 0123456789abcdef, guest fedcba9876543210",
            "fedcba9876543210",
        );
        assert!(
            m.contains("012345") && m.contains("fedcba") && m.contains("same bopl battle build"),
            "{m}"
        );
        let m = super::reject_message(
            "version mismatch: host runs bopl-port 0.0.9 net1 (protocol 2)",
            "x",
        );
        assert!(m.contains("0.0.9") && m.contains("same version"), "{m}");
    }
}
