//! The in-game room browser (main menu → online → room list): public
//! bopl-port rooms from Warpzone's room list (`NetSession::list_rooms`),
//! refreshed every few seconds, with the player count and whether the
//! room runs our version. Choosing a room joins it by id
//! (`OnlineIntent::Join`); the browser's own Warpzone session is dropped.

use bopl_net::warpzone_core::client::Event;
use bopl_net::warpzone_core::protocol::api::RoomInfo;
use bopl_net::{NetConfig, NetEvent, NetSession};

use super::adapter::NetWorld;

/// Seconds between automatic refreshes.
const REFRESH: f32 = 4.0;
/// Room rows per page.
pub const PAGE: usize = 6;

/// One listed room.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomRow {
    pub id: String,
    /// Shown text (lower case: the menu font draws capitals).
    pub label: String,
    /// The room runs another bopl-port version (joining would be refused).
    pub other_version: bool,
}

pub struct Browser {
    net: Option<NetSession<NetWorld>>,
    pub rooms: Vec<RoomRow>,
    pub page: usize,
    /// Status line (connecting, errors, page).
    pub status: String,
    logged_in: bool,
    since_list: f32,
    /// A list arrived at least once.
    listed: bool,
}

/// The row for a Warpzone room.
pub fn room_row(r: &RoomInfo, version: &str) -> RoomRow {
    let players = r
        .metadata
        .get("players")
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(r.players);
    let mode = r.metadata.get("mode").cloned().unwrap_or_default();
    let other = r.game_version != version;
    let mut label = format!("{}  {players}/{}", r.name, r.max_players.max(players));
    if !mode.is_empty() && !r.name.to_lowercase().contains(&mode.to_lowercase()) {
        label = format!("{label}  {mode}");
    }
    if other {
        label = format!("{label}  (other version)");
    }
    RoomRow {
        id: r.id.clone(),
        label: label.to_lowercase(),
        other_version: other,
    }
}

impl Browser {
    /// Connects to Warpzone (the identity of `WARPZONE_HOME`; `name`
    /// registers one if there is none yet).
    pub fn new(name: &str) -> Self {
        let mut cfg = NetConfig::new(&super::net_version());
        cfg.auto_register = Some(name.to_string());
        let (net, status) = match NetSession::new(cfg) {
            Ok(n) => (Some(n), "connecting to warpzone...".to_string()),
            Err(e) => (None, format!("warpzone: {e}")),
        };
        Self {
            net,
            rooms: Vec::new(),
            page: 0,
            status,
            logged_in: false,
            since_list: 0.0,
            listed: false,
        }
    }

    /// Asks for the list now.
    pub fn refresh(&mut self) {
        if let Some(n) = self.net.as_mut()
            && self.logged_in
        {
            n.list_rooms(true);
            self.since_list = 0.0;
        }
    }

    pub fn pages(&self) -> usize {
        self.rooms.len().div_ceil(PAGE).max(1)
    }

    /// Rooms of the current page.
    pub fn page_rooms(&self) -> &[RoomRow] {
        let from = (self.page * PAGE).min(self.rooms.len());
        let to = (from + PAGE).min(self.rooms.len());
        &self.rooms[from..to]
    }

    pub fn turn_page(&mut self, dir: i32) {
        let n = self.pages() as i32;
        self.page = (self.page as i32 + dir).rem_euclid(n) as usize;
        self.update_status();
    }

    fn update_status(&mut self) {
        if !self.listed {
            return;
        }
        self.status = if self.rooms.is_empty() {
            "no open rooms: host one, or refresh".into()
        } else if self.pages() > 1 {
            format!(
                "{} rooms · page {}/{} (left/right)",
                self.rooms.len(),
                self.page + 1,
                self.pages()
            )
        } else {
            format!(
                "{} room{}",
                self.rooms.len(),
                if self.rooms.len() == 1 { "" } else { "s" }
            )
        };
    }

    /// Polls Warpzone (every menu tick).
    pub fn tick(&mut self, dt: f32) {
        let Some(mut net) = self.net.take() else {
            return;
        };
        net.poll();
        let version = super::net_version();
        while let Some(e) = net.next_event() {
            let NetEvent::Warpzone(b) = e else { continue };
            match *b {
                Event::LoggedIn {
                    account: Some(_), ..
                }
                | Event::Registered(_) => {
                    self.logged_in = true;
                    self.status = "listing rooms...".into();
                    net.list_rooms(true);
                    self.since_list = 0.0;
                }
                Event::RoomList(list) => {
                    self.rooms = list
                        .iter()
                        .filter(|r| r.game == bopl_net::GAME_ID)
                        .map(|r| room_row(r, &version))
                        .collect();
                    self.page = self.page.min(self.pages() - 1);
                    self.listed = true;
                    self.update_status();
                }
                Event::Error { op, message, .. } => {
                    if op == "register" {
                        // Name taken: another one.
                        let n = format!("bopl{}", std::process::id() % 100_000);
                        net.register(&n);
                    } else {
                        self.status = format!("error: {message}");
                    }
                }
                _ => {}
            }
        }
        self.net = Some(net);
        self.since_list += dt;
        if self.logged_in && self.since_list > REFRESH {
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows() {
        let mut r: RoomInfo = serde_json::from_value(serde_json::json!({
            "id": "v7thncejf4", "game": "bopl-port", "game_version": "x",
            "name": "Ann's Free-for-all", "host": "Ann", "players": 2, "max_players": 12,
            "visibility": "public", "entry": "open", "audience": {}, "ranked": false,
            "staged": false, "metadata": {}, "members": ["Ann", "Bo"], "created": 0
        }))
        .unwrap();
        let row = room_row(&r, "x");
        assert_eq!(row.label, "ann's free-for-all  2/12");
        assert!(!row.other_version);
        r.metadata.insert("players".into(), "5".into());
        r.metadata.insert("mode".into(), "2 teams".into());
        let row = room_row(&r, "y");
        assert_eq!(
            row.label,
            "ann's free-for-all  5/12  2 teams  (other version)"
        );
        r.metadata.insert("mode".into(), "free-for-all".into());
        assert_eq!(room_row(&r, "x").label, "ann's free-for-all  5/12");
        assert!(row.other_version);
    }
}
