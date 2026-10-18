//! State shared by the screens of the app: game data, GPU assets, sounds,
//! sim content and level cache, fonts, settings.

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use bopl_import::GameData;
use bopl_render::{FontId, Gpu, Renderer};
use serde::{Deserialize, Serialize};

use crate::assets::Assets;
use crate::match_view::MatchVisuals;
use crate::sim_match::LevelCache;
use crate::sound::Sounds;

/// How the window covers the screen (ui.md 3.1 "screen").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenMode {
    Windowed,
    /// Borderless fullscreen at the desktop resolution (the original's
    /// default).
    #[default]
    Borderless,
    /// Exclusive fullscreen at `Settings::resolution`.
    Fullscreen,
}

impl ScreenMode {
    pub const ALL: [ScreenMode; 3] = [
        ScreenMode::Windowed,
        ScreenMode::Borderless,
        ScreenMode::Fullscreen,
    ];
}

/// Controller rumble (ui.md 3.1): off, on, high (motor speeds × 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rumble {
    Off,
    #[default]
    On,
    High,
}

/// Streamer mode (ui.md 3.1 "hide"): online names shown as "player N".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hide {
    #[default]
    Nothing,
    Names,
    #[serde(rename = "names and avatars")]
    NamesAndAvatars,
}

/// The display options the window host applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub mode: ScreenMode,
    /// Window size (windowed) / display mode (exclusive fullscreen);
    /// `None` = the default (1280x720 windowed, the desktop mode).
    pub resolution: Option<(u32, u32)>,
    pub vsync: bool,
}

/// Persisted options (match-flow.md 2, ui.md 3.1), `settings.json` in the
/// user data dir.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Ability slots per player (1..3).
    pub number_of_abilities: usize,
    /// 0..10.
    pub sfx_volume: u32,
    pub music_volume: u32,
    /// 0..10 (ours: the original's master volume is fixed at full; no
    /// menu row, `settings.json` only).
    pub master_volume: u32,
    pub screen_shake: bool,
    pub rumble: Rumble,
    pub vsync: bool,
    pub screen: ScreenMode,
    /// Window size / fullscreen display mode; `None` = default.
    pub resolution: Option<(u32, u32)>,
    pub hide: Hide,
    /// Ours (docs/STATUS.md decisions): with more than 4 players the camera
    /// zooms out to keep the players in view. Off = the original's fixed
    /// camera.
    pub camera_zoom: bool,
    /// Index into the localisation's languages (0 English).
    pub language: usize,
    /// Remapped keys and buttons per device type (ui.md 6).
    pub bindings: crate::input::Bindings,
    /// Online player name (empty: the Warpzone account name).
    pub player_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            number_of_abilities: 3,
            sfx_volume: 10,
            music_volume: 7,
            master_volume: 10,
            screen_shake: true,
            rumble: Rumble::On,
            // The original's defaults: vsync off, borderless; we start
            // with vsync on (no tearing, no busy loop).
            vsync: true,
            screen: ScreenMode::Borderless,
            resolution: None,
            hide: Hide::Nothing,
            camera_zoom: false,
            language: 0,
            bindings: Default::default(),
            player_name: String::new(),
        }
    }
}

impl Settings {
    /// The display part.
    pub fn display(&self) -> Display {
        Display {
            mode: self.screen,
            resolution: self.resolution,
            vsync: self.vsync,
        }
    }

    fn path() -> Option<PathBuf> {
        bopl_import::paths::data_dir().map(|d| d.join("settings.json"))
    }

    /// Loads the settings file (defaults when missing or broken).
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&p, serde_json::to_vec_pretty(self).unwrap_or_default())
            {
                log::warn!("saving {}: {e}", p.display());
            }
        }
    }
}

impl Shared {
    /// Stores new bindings (persisted unless headless) for the input hub.
    pub fn set_bindings(&mut self, b: crate::input::Bindings) {
        self.settings.bindings = b;
        self.bindings_changed = true;
        if !self.headless {
            self.settings.save();
        }
    }
}

/// Everything the screens share.
pub struct Shared {
    pub data: Rc<GameData>,
    pub assets: Assets,
    pub sounds: Sounds,
    pub visuals: Rc<MatchVisuals>,
    pub levels: Rc<LevelCache>,
    pub settings: Settings,
    pub fonts: FontCache,
    /// Headless run (no persistent settings writes, deterministic).
    pub headless: bool,
    /// Seconds spent in menus (`CircleBG.TimeSpentInMenus`: the circle
    /// pattern keeps moving across menu screens).
    pub menu_time: f32,
    /// `settings.bindings` changed: the app hands them to the input hub.
    pub bindings_changed: bool,
    /// The display options changed (Save in the settings): the app hands
    /// them to the window host.
    pub display_changed: bool,
    /// What the window host reports (display modes, controllers).
    pub platform: crate::app::PlatformInfo,
    /// Controller rumble requests for the window host (device, low, high,
    /// milliseconds).
    pub rumble: Vec<(crate::input::DeviceId, f32, f32, u32)>,
}

impl Shared {
    pub fn new(data: Rc<GameData>, headless: bool) -> Result<Self> {
        let levels = Rc::new(LevelCache::new(data.clone())?);
        let visuals = Rc::new(MatchVisuals::new(data.clone(), levels.content.clone()));
        let settings = if headless {
            Settings::default()
        } else {
            Settings::load()
        };
        let mut sounds = Sounds::new(data.clone());
        sounds.sfx_volume = settings.sfx_volume as f32;
        sounds.music_volume = settings.music_volume as f32;
        Ok(Self {
            assets: Assets::new(data.clone()),
            sounds,
            visuals,
            levels,
            settings,
            fonts: FontCache::default(),
            headless,
            menu_time: 0.0,
            bindings_changed: true,
            display_changed: false,
            platform: Default::default(),
            rumble: Vec::new(),
            data,
        })
    }

    /// A TextMeshPro font by asset name (cached).
    #[allow(dead_code)]
    pub fn font(&mut self, gpu: &Gpu, renderer: &mut Renderer, name: &str) -> Option<FontId> {
        self.fonts.get(gpu, renderer, &mut self.assets, name)
    }
}

/// TextMeshPro fonts registered with the renderer, by asset name.
#[derive(Default)]
pub struct FontCache {
    fonts: Vec<(String, Option<FontId>)>,
}

impl FontCache {
    /// The font (with the overlay fallbacks for the main UI font).
    pub fn get(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut Assets,
        name: &str,
    ) -> Option<FontId> {
        if let Some((_, f)) = self.fonts.iter().find(|(n, _)| n == name) {
            return *f;
        }
        let f = if name == crate::level_scene::UI_FONT {
            crate::level_scene::ui_font(assets, gpu, renderer)
        } else {
            assets.font(gpu, renderer, name)
        };
        if f.is_none() {
            log::warn!("font {name:?} not available");
        }
        self.fonts.push((name.to_string(), f));
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_files_from_older_versions_load_with_defaults() {
        let s: Settings = serde_json::from_str(r#"{"sfx_volume": 4, "language": 2}"#).unwrap();
        assert_eq!(s.sfx_volume, 4);
        assert_eq!(s.language, 2);
        assert_eq!(s.screen, ScreenMode::Borderless);
        assert_eq!(s.rumble, Rumble::On);
        assert!(!s.camera_zoom);
        assert_eq!(s.master_volume, 10);
    }

    #[test]
    fn settings_round_trip() {
        let s = Settings {
            screen: ScreenMode::Fullscreen,
            resolution: Some((1600, 900)),
            vsync: false,
            rumble: Rumble::High,
            hide: Hide::NamesAndAvatars,
            camera_zoom: true,
            ..Settings::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"fullscreen\"") && j.contains("\"names and avatars\""));
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
        assert_eq!(
            back.display(),
            Display {
                mode: ScreenMode::Fullscreen,
                resolution: Some((1600, 900)),
                vsync: false
            }
        );
    }
}
