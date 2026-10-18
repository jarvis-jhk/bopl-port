//! Loading an import dir back into memory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;

use crate::format::{self, FORMAT_VERSION, ObjId};

/// Build index of the tutorial scene (a non-level scene that
/// [`GameData::level`] also returns as a level).
pub const TUTORIAL: usize = 5;

/// The index files of an import. Bulk data (PNGs, Oggs, levels, scenes,
/// prefabs) is loaded on demand.
#[derive(Clone, Debug)]
pub struct GameData {
    /// The import dir.
    pub dir: PathBuf,
    /// `index.json`.
    pub index: format::Index,
    /// `textures.json`.
    pub textures: BTreeMap<ObjId, format::Texture>,
    /// `sprites.json`.
    pub sprites: BTreeMap<ObjId, format::Sprite>,
    /// `audio.json`.
    pub audio: BTreeMap<ObjId, format::AudioClip>,
    /// `materials.json`.
    pub materials: BTreeMap<ObjId, format::Material>,
    /// `animations.json`.
    pub animations: format::Animations,
    /// `fonts.json`.
    pub fonts: Vec<format::Font>,
    /// `font_files.json`.
    pub font_files: BTreeMap<ObjId, format::FontFile>,
    /// `player.json`.
    pub player: format::Player,
    /// `abilities.json`.
    pub abilities: format::Abilities,
    /// `globals.json`.
    pub globals: format::Globals,
    /// `localization.json`.
    pub localization: format::Localization,
    /// `assets.json`: ScriptableObjects (`AnimationData`, colour lists,
    /// input actions, ...).
    pub assets: BTreeMap<ObjId, format::Asset>,
    /// `prefab_index.json`.
    pub prefabs: format::PrefabIndex,
}

/// Reads and parses a JSON file.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let s = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&s).with_context(|| format!("parsing {}", path.display()))
}

impl GameData {
    /// Loads an import dir; fails on a format version mismatch.
    pub fn load(dir: impl AsRef<Path>) -> Result<GameData> {
        let dir = dir.as_ref().to_path_buf();
        let index: format::Index = read(&dir.join("index.json"))?;
        if index.format_version != FORMAT_VERSION {
            bail!(
                "{}: import format {} but this build reads {FORMAT_VERSION}; re-import",
                dir.display(),
                index.format_version
            );
        }
        let r = |name: &str| dir.join(name);
        Ok(GameData {
            textures: read(&r("textures.json"))?,
            sprites: read(&r("sprites.json"))?,
            audio: read(&r("audio.json"))?,
            materials: read(&r("materials.json"))?,
            animations: read(&r("animations.json"))?,
            fonts: read(&r("fonts.json"))?,
            font_files: read(&r("font_files.json"))?,
            player: read(&r("player.json"))?,
            abilities: read(&r("abilities.json"))?,
            globals: read(&r("globals.json"))?,
            localization: read(&r("localization.json"))?,
            prefabs: read(&r("prefab_index.json"))?,
            assets: read(&r("assets.json"))?,
            index,
            dir,
        })
    }

    /// Build indices of the playable levels, in build order.
    pub fn level_indices(&self) -> Vec<usize> {
        self.index
            .scenes
            .iter()
            .filter(|s| s.level && s.file.is_some())
            .map(|s| s.build_index)
            .collect()
    }

    fn scene_file(&self, build_index: usize, level: bool) -> Result<PathBuf> {
        let e = self
            .index
            .scenes
            .iter()
            .find(|s| s.build_index == build_index)
            .with_context(|| format!("no scene {build_index}"))?;
        if e.level != level {
            bail!(
                "scene {build_index} ({}) is {}a level",
                e.name,
                if e.level { "" } else { "not " }
            );
        }
        let f = e.file.as_ref().context("scene was not imported")?;
        Ok(self.dir.join(f))
    }

    /// Loads a level by build index ([`TUTORIAL`] gives the tutorial scene
    /// as a level, see [`GameData::tutorial_level`]).
    pub fn level(&self, build_index: usize) -> Result<format::Level> {
        if build_index == TUTORIAL {
            return self.tutorial_level();
        }
        read(&self.scene_file(build_index, true)?)
    }

    /// The tutorial scene (docs/spec/tutorial.md) in the shape of a level,
    /// built at load time: `session` = the `TutorialGameHandler` fields, the
    /// four team spawns = its `playerSpawn`, grass, `DetPhysics`, the main
    /// camera, platforms and shapes picked like the importer does.
    pub fn tutorial_level(&self) -> Result<format::Level> {
        let sc = self.scene(TUTORIAL)?;
        let objects = sc.objects;
        let comp = |class: &str| {
            objects
                .iter()
                .enumerate()
                .find_map(|(i, o)| o.component(class).map(|c| (i, c)))
        };
        let session = comp("TutorialGameHandler")
            .map(|(_, c)| c.fields.clone())
            .context("tutorial scene without TutorialGameHandler")?;
        let raw = |j: &serde_json::Value| j["m_rawValue"].as_i64().unwrap_or(0);
        let spawn = [
            raw(&session["playerSpawn"]["x"]),
            raw(&session["playerSpawn"]["y"]),
        ];
        let with = |pred: &dyn Fn(&format::Object) -> bool| -> Vec<usize> {
            objects
                .iter()
                .enumerate()
                .filter(|(_, o)| pred(o))
                .map(|(i, _)| i)
                .collect()
        };
        Ok(format::Level {
            build_index: TUTORIAL,
            name: sc.name.clone(),
            path: sc.path.clone(),
            level_type: 0,
            team_spawns: vec![spawn; 4],
            teammate_spawn_spacing: 0,
            physics: comp("DetPhysics").map_or(serde_json::Value::Null, |(_, c)| c.fields.clone()),
            main_camera: comp("PlayerAverageCamera").map(|(i, _)| i),
            ability_spawner: None,
            platforms: with(&|o| o.component("StickyRoundedRectangle").is_some()),
            shapes: with(&|o| o.shape.is_some()),
            session,
            objects,
        })
    }

    /// Loads a non-level scene (0..=5) by build index.
    pub fn scene(&self, build_index: usize) -> Result<format::Scene> {
        read(&self.scene_file(build_index, false)?)
    }

    /// Loads a prefab by its root GameObject ID, or by the ID of any
    /// GameObject or component inside it.
    pub fn prefab(&self, id: &str) -> Result<format::Prefab> {
        let root = self.prefabs.objects.get(id).map_or(id, String::as_str);
        read(&self.dir.join(format::prefab_file(root)))
    }

    /// Absolute path of a texture's PNG.
    pub fn texture_path(&self, id: &str) -> Option<PathBuf> {
        self.textures.get(id).map(|t| self.dir.join(&t.png))
    }

    /// Absolute path of an audio clip's Ogg file.
    pub fn audio_path(&self, id: &str) -> Option<PathBuf> {
        self.audio.get(id).map(|a| self.dir.join(&a.file))
    }

    /// The first audio clip with the given name (`AudioClip.m_Name`).
    pub fn audio_by_name(&self, name: &str) -> Option<(&ObjId, &format::AudioClip)> {
        self.audio.iter().find(|(_, a)| a.name == name)
    }
}

/// Decodes a PNG written by the importer into RGBA8 (top row first).
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let f = fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info()?;
    let mut buf = vec![0; r.output_buffer_size().context("png too large")?];
    let info = r.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("{}: indexed PNG after expansion", path.display()),
    };
    Ok((info.width, info.height, rgba))
}
