//! The import run: game data dir → import dir (see `docs/formats/import.md`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value as J;
use unity_assets::classes::{self, AudioClip, Sprite, Texture2D};
use unity_assets::{AssetDir, AssetFile, texture};

use crate::format::{self, FORMAT_REVISION, FORMAT_VERSION, ObjId};
use crate::scene::{Ctx, obj_id, ref_id, script_fields, to_json};

/// How far prefab references are followed from the scenes and the global
/// assets (ability -> projectile -> effect -> ... chains are short; the
/// limit only guards against pathological data).
const PREFAB_DEPTH: usize = 8;

/// Singleton script components recorded in `globals.json` (first
/// occurrence in build order).
const SINGLETONS: &[&str] = &[
    "AbilityGrid",
    "AbilitySelectController",
    "AudioManager",
    "CharacterSelectHandler",
    "CharacterSelectHandler_online",
    "CharacterStatsList",
    "MainMenu",
    "MidGameAbilitySelect",
    "SettingsInitializer",
    "SteamManager",
    "TutorialGameHandler",
    "WinnerTriangleCanvas",
];

/// Import options.
#[derive(Clone, Debug)]
pub struct Options {
    /// Game directory (with `*_Data`) or data directory.
    pub game_dir: PathBuf,
    /// Output directory; replaced atomically.
    pub out_dir: PathBuf,
    /// Re-import even if the output is up to date.
    pub force: bool,
}

/// What an import did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Output was already up to date; nothing was written.
    pub up_to_date: bool,
    /// Objects written per kind.
    pub counts: BTreeMap<String, usize>,
    /// Non-fatal problems.
    pub warnings: Vec<String>,
    /// Wall time.
    pub seconds: f64,
}

type Files = [Arc<AssetFile>];

/// An exported prefab and the asset GameObjects it references.
type PrefabOut = (format::Prefab, BTreeSet<ObjId>);

struct Run<'a> {
    ctx: Ctx<'a>,
    dir: &'a AssetDir,
    out: PathBuf,
    /// The previous import (textures and audio whose source is unchanged
    /// are moved over instead of being encoded again).
    old: Option<OldImport>,
    warnings: Mutex<Vec<String>>,
}

/// Reusable outputs of a previous import of the same format version.
struct OldImport {
    dir: PathBuf,
    textures: BTreeMap<ObjId, format::Texture>,
    audio: BTreeMap<ObjId, format::AudioClip>,
    font_files: BTreeMap<ObjId, format::FontFile>,
}

impl OldImport {
    fn load(dir: &Path) -> Option<OldImport> {
        let idx: format::Index =
            serde_json::from_slice(&fs::read(dir.join("index.json")).ok()?).ok()?;
        if idx.format_version != FORMAT_VERSION {
            return None;
        }
        let read = |n: &str| fs::read(dir.join(n)).ok();
        Some(OldImport {
            dir: dir.to_path_buf(),
            textures: serde_json::from_slice(&read("textures.json")?).ok()?,
            audio: serde_json::from_slice(&read("audio.json")?).ok()?,
            font_files: read("font_files.json")
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
        })
    }

    /// Moves `rel` from the old import to `to` if the old entry had the same
    /// source hash; true on success.
    fn reuse(&self, rel: &str, old_hash: Option<&str>, hash: &str, to: &Path) -> bool {
        old_hash == Some(hash) && fs::rename(self.dir.join(rel), to).is_ok()
    }
}

impl Run<'_> {
    fn warn(&self, msg: String) {
        log::warn!("{msg}");
        self.warnings.lock().unwrap().push(msg);
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T, pretty: bool) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut w = BufWriter::new(fs::File::create(path).with_context(|| path.display().to_string())?);
    if pretty {
        serde_json::to_writer_pretty(&mut w, v)?;
    } else {
        serde_json::to_writer(&mut w, v)?;
    }
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let f = BufWriter::new(fs::File::create(path)?);
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// 64-bit FNV-1a, used as the source hash of textures and audio clips.
fn fnv64(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        // Process 8 bytes at a time for speed; the hash only has to detect
        // changed sources, not resist attacks.
        let (chunks, rest) = p.as_chunks::<8>();
        for c in chunks {
            h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0x100_0000_01b3);
        }
        for &b in rest {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        h = (h ^ p.len() as u64).wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Fingerprint of the data dir: names, sizes and modification times.
pub fn fingerprint(dir: &AssetDir) -> Result<String> {
    let mut entries: Vec<(String, u64, u64)> = Vec::new();
    for sub in [
        dir.data_dir().to_path_buf(),
        dir.data_dir().join("Resources"),
    ] {
        let Ok(rd) = fs::read_dir(&sub) else { continue };
        for e in rd {
            let e = e?;
            let m = e.metadata()?;
            if m.is_file() {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                entries.push((e.file_name().to_string_lossy().into_owned(), m.len(), mtime));
            }
        }
    }
    entries.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (n, l, t) in &entries {
        for b in format!("{n}:{l}:{t};").bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    Ok(format!("{h:016x}-{}", entries.len()))
}

/// Whether `dir` holds an import that [`import`] would keep as is for the
/// game at `game_dir` (same format version, at least the current revision,
/// same source fingerprint).
pub fn is_up_to_date(game_dir: &Path, dir: &Path) -> bool {
    let Ok(assets) = AssetDir::open(game_dir) else {
        return false;
    };
    let Ok(fp) = fingerprint(&assets) else {
        return false;
    };
    up_to_date_index(dir, &fp).is_some()
}

// FORMAT_REVISION is 0 for now; the check matters once it grows.
#[allow(clippy::absurd_extreme_comparisons)]
fn up_to_date_index(dir: &Path, fp: &str) -> Option<format::Index> {
    let s = fs::read(dir.join("index.json")).ok()?;
    let idx: format::Index = serde_json::from_slice(&s).ok()?;
    (idx.format_version == FORMAT_VERSION
        && idx.format_revision >= FORMAT_REVISION
        && idx.source_fingerprint == fp)
        .then_some(idx)
}

/// Runs an import.
pub fn import(opts: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let dir = AssetDir::open(&opts.game_dir)?;
    let fp = fingerprint(&dir)?;
    let index_path = opts.out_dir.join("index.json");
    if !opts.force
        && let Some(idx) = up_to_date_index(&opts.out_dir, &fp)
    {
        return Ok(Report {
            up_to_date: true,
            counts: idx.counts,
            seconds: t0.elapsed().as_secs_f64(),
            ..Default::default()
        });
    }
    if opts.out_dir.exists()
        && !index_path.exists()
        && fs::read_dir(&opts.out_dir)?.next().is_some()
    {
        bail!(
            "{} exists, is not empty and is not an import dir; refusing to replace it",
            opts.out_dir.display()
        );
    }
    let tmp = opts.out_dir.with_extension("tmp");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    for sub in ["textures", "audio", "levels", "scenes", "prefabs", "fonts"] {
        fs::create_dir_all(tmp.join(sub))?;
    }
    let run = Run {
        ctx: Ctx::new(&dir)?,
        dir: &dir,
        out: tmp.clone(),
        old: OldImport::load(&opts.out_dir),
        warnings: Default::default(),
    };
    let files: Vec<_> = dir
        .serialized_file_names()?
        .iter()
        .map(|n| dir.file(n))
        .collect::<unity_assets::Result<_>>()?;
    let assets: Vec<_> = files
        .iter()
        .filter(|f| !f.name.starts_with("level"))
        .cloned()
        .collect();
    let mut counts = BTreeMap::new();

    // Independent bulk work in parallel: textures, audio, scenes.
    let ((textures, audio), scenes) = rayon::join(
        || rayon::join(|| run.textures(&files), || run.audio(&files)),
        || run.scenes(),
    );
    let (textures, audio, (scenes, scene_refs)) = (textures?, audio?, scenes?);
    counts.insert("textures".into(), textures.len());
    write_json(&tmp.join("textures.json"), &textures, false)?;
    counts.insert("audio".into(), audio.len());
    write_json(&tmp.join("audio.json"), &audio, false)?;
    counts.insert(
        "levels".into(),
        scenes
            .iter()
            .filter(|s| s.level && s.file.is_some())
            .count(),
    );
    counts.insert(
        "scenes".into(),
        scenes
            .iter()
            .filter(|s| !s.level && s.file.is_some())
            .count(),
    );

    let sprites = run.sprites(&files, &textures)?;
    counts.insert("sprites".into(), sprites.len());
    write_json(&tmp.join("sprites.json"), &sprites, false)?;

    let globals = run.globals(&assets, &scenes)?;
    counts.insert("sprite_lists".into(), globals.sprite_lists.len());
    write_json(&tmp.join("globals.json"), &globals, false)?;

    let (abilities, mut queue) = run.abilities(&globals)?;
    counts.insert("abilities".into(), abilities.full.len());
    write_json(&tmp.join("abilities.json"), &abilities, false)?;

    let player = run.player(&scenes)?;
    queue.extend(
        [
            &player.prefab,
            &player.input_updater,
            &player.ability_pickup,
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    write_json(&tmp.join("player.json"), &player, false)?;

    queue.extend(scene_refs);
    for a in [
        &globals.player_colors,
        &globals.team_colors,
        &globals.beam_colors,
    ]
    .into_iter()
    .flatten()
    {
        crate::collect_refs(&a.fields, &mut queue);
    }
    let prefab_index = run.prefab_closure(queue)?;
    counts.insert("prefabs".into(), prefab_index.roots.len());
    write_json(&tmp.join("prefab_index.json"), &prefab_index, false)?;

    let script_assets = run.script_assets(&assets)?;
    counts.insert("assets".into(), script_assets.len());
    write_json(&tmp.join("assets.json"), &script_assets, false)?;

    let localization = run.localization(&assets)?;
    counts.insert(
        "localized_strings".into(),
        localization.strings.get("en").map_or(0, Vec::len),
    );
    write_json(&tmp.join("localization.json"), &localization, false)?;

    let materials = run.materials(&files)?;
    counts.insert("materials".into(), materials.len());
    write_json(&tmp.join("materials.json"), &materials, false)?;

    let (animations, anim_warnings) = crate::anim::animations(&dir, &files);
    for w in anim_warnings {
        run.warn(format!("animation: {w}"));
    }
    counts.insert("animation_clips".into(), animations.clips.len());
    counts.insert("animator_controllers".into(), animations.controllers.len());
    write_json(&tmp.join("animations.json"), &animations, false)?;

    let fonts = run.fonts(&assets)?;
    counts.insert("fonts".into(), fonts.len());
    write_json(&tmp.join("fonts.json"), &fonts, false)?;
    let font_files = run.font_files(&assets)?;
    counts.insert("font_files".into(), font_files.len());
    write_json(&tmp.join("font_files.json"), &font_files, false)?;

    let ggm = dir.file("globalgamemanagers")?;
    let tm = ggm
        .objects_of(78)
        .next()
        .map(|o| {
            dir.read(&ggm, o.path_id)
                .map(|v| classes::TagManager::from_value(&v))
        })
        .transpose()?
        .unwrap_or_default();
    let index = format::Index {
        format_version: FORMAT_VERSION,
        format_revision: FORMAT_REVISION,
        unity_version: ggm.meta.unity_version.clone(),
        source_fingerprint: fp,
        scenes,
        sorting_layers: tm
            .sorting_layers
            .iter()
            .map(|l| format::SortingLayer {
                name: l.name.clone(),
                id: l.unique_id as u32,
            })
            .collect(),
        layers: tm.layers.clone(),
        tags: run.ctx.tags().to_vec(),
        settings: run.settings(&ggm)?,
        player_prefab: player.prefab.clone(),
        counts: counts.clone(),
    };
    write_json(&tmp.join("index.json"), &index, true)?;

    drop(run.old);
    if opts.out_dir.exists() {
        fs::remove_dir_all(&opts.out_dir)?;
    }
    fs::rename(&tmp, &opts.out_dir)?;
    Ok(Report {
        up_to_date: false,
        counts,
        warnings: run.warnings.into_inner().unwrap(),
        seconds: t0.elapsed().as_secs_f64(),
    })
}

impl Run<'_> {
    fn textures(&self, files: &Files) -> Result<BTreeMap<ObjId, format::Texture>> {
        let items: Vec<_> = files
            .iter()
            .flat_map(|f| f.objects_of(28).map(move |o| (f.clone(), o.path_id)))
            .collect();
        let out: Vec<_> = items
            .par_iter()
            .filter_map(|(f, pid)| {
                let id = obj_id(f, *pid);
                match self.texture(f, *pid, &id) {
                    Ok(t) => t.map(|t| (id, t)),
                    Err(e) => {
                        self.warn(format!("texture {id}: {e:#}"));
                        None
                    }
                }
            })
            .collect();
        Ok(out.into_iter().collect())
    }

    fn texture(&self, f: &AssetFile, pid: i64, id: &str) -> Result<Option<format::Texture>> {
        let t = Texture2D::from_value(&self.dir.read(f, pid)?);
        if t.width == 0 || t.height == 0 {
            return Ok(None);
        }
        let stream;
        let data: &[u8] = if t.image_data.is_empty() {
            stream = self
                .dir
                .stream(&t.stream.path, t.stream.offset, t.stream.size)?;
            &stream
        } else {
            &t.image_data
        };
        let level0 = texture::level0_size(t.format, t.width, t.height)
            .map_or(data, |n| &data[..n.min(data.len())]);
        let dims = format!("{}x{}:{}", t.width, t.height, t.format);
        let hash = fnv64(&[dims.as_bytes(), level0]);
        let png = format!("textures/{}.png", format::sanitize(id));
        let dest = self.out.join(&png);
        let reused = self.old.as_ref().is_some_and(|o| {
            let old = o.textures.get(id);
            old.is_some_and(|t| t.png == png)
                && o.reuse(
                    &png,
                    old.and_then(|t| t.source_hash.as_deref()),
                    &hash,
                    &dest,
                )
        });
        if !reused {
            let rgba = texture::decode(t.format, t.width, t.height, data)?;
            write_png(&dest, t.width, t.height, &rgba)?;
        }
        Ok(Some(format::Texture {
            name: t.name,
            png,
            width: t.width,
            height: t.height,
            format: texture::format_name(t.format).to_owned(),
            filter_mode: t.filter_mode,
            wrap_mode: t.wrap_u,
            source_hash: Some(hash),
        }))
    }

    fn sprites(
        &self,
        files: &Files,
        textures: &BTreeMap<ObjId, format::Texture>,
    ) -> Result<BTreeMap<ObjId, format::Sprite>> {
        let mut out = BTreeMap::new();
        for f in files {
            for o in f.objects_of(213) {
                let id = obj_id(f, o.path_id);
                let s = match self.dir.read(f, o.path_id) {
                    Ok(v) => Sprite::from_value(&v),
                    Err(e) => {
                        self.warn(format!("sprite {id}: {e}"));
                        continue;
                    }
                };
                let tex = ref_id(f, s.texture);
                let th = tex
                    .as_ref()
                    .and_then(|t| textures.get(t))
                    .map_or(0.0, |t| t.height as f32);
                let [x, y, w, h] = s.texture_rect;
                let ppu = if s.pixels_to_units > 0.0 {
                    s.pixels_to_units
                } else {
                    100.0
                };
                out.insert(
                    id,
                    format::Sprite {
                        name: s.name.clone(),
                        texture: tex,
                        rect: s.rect,
                        texture_rect: [x, th - (y + h), w, h],
                        texture_rect_offset: s.texture_rect_offset,
                        pivot: s.pivot,
                        pixels_per_unit: ppu,
                        border: s.border,
                        packed: s.packed(),
                        packing_rotation: s.packing_rotation(),
                        physics_shape: s
                            .physics_shape
                            .iter()
                            .map(|p| p.iter().map(|q| [q[0] / ppu, q[1] / ppu]).collect())
                            .collect(),
                    },
                );
            }
        }
        Ok(out)
    }

    fn audio(&self, files: &Files) -> Result<BTreeMap<ObjId, format::AudioClip>> {
        // Names first (sequential, deterministic de-duplication), then convert.
        let mut used = HashSet::new();
        let mut items = Vec::new();
        for f in files {
            for o in f.objects_of(83) {
                let id = obj_id(f, o.path_id);
                let clip = match self.dir.read(f, o.path_id) {
                    Ok(v) => AudioClip::from_value(&v),
                    Err(e) => {
                        self.warn(format!("audio {id}: {e}"));
                        continue;
                    }
                };
                let base = format::sanitize(&clip.name);
                let mut name = base.clone();
                let mut n = 1;
                while !used.insert(name.to_lowercase()) {
                    n += 1;
                    name = format!("{base}_{n}");
                }
                items.push((id, clip, format!("audio/{name}.ogg")));
            }
        }
        let out: Vec<_> = items
            .into_par_iter()
            .filter_map(|(id, clip, file)| {
                let r = &clip.resource;
                let res = self
                    .dir
                    .stream(&r.path, r.offset, r.size)
                    .map_err(anyhow::Error::from)
                    .and_then(|bank| {
                        let hash = fnv64(&[&bank]);
                        let dest = self.out.join(&file);
                        if let Some(o) = &self.old
                            && let Some(old) = o.audio.get(&id)
                            && old.file == file
                            && o.reuse(&file, old.source_hash.as_deref(), &hash, &dest)
                        {
                            return Ok((old.samples, hash));
                        }
                        let (ogg, samples) = unity_assets::audio::fsb5_to_ogg(&bank)?;
                        fs::write(dest, ogg)?;
                        Ok((samples, hash))
                    });
                match res {
                    Ok((samples, hash)) => Some((
                        id,
                        format::AudioClip {
                            name: clip.name,
                            file,
                            channels: clip.channels,
                            frequency: clip.frequency,
                            length: clip.length,
                            samples,
                            source_hash: Some(hash),
                        },
                    )),
                    Err(e) => {
                        self.warn(format!("audio {id} ({}): {e:#}", clip.name));
                        None
                    }
                }
            })
            .collect();
        Ok(out.into_iter().collect())
    }

    /// Root GameObjects of a scene file, in file order.
    fn scene_roots(&self, f: &AssetFile) -> Result<Vec<i64>> {
        let mut roots = Vec::new();
        for o in f
            .meta
            .objects
            .iter()
            .filter(|o| o.class_id == 4 || o.class_id == 224)
        {
            let t = classes::Transform::from_value(&self.dir.read(f, o.path_id)?);
            if t.father.is_null() && !t.game_object.is_null() {
                roots.push(t.game_object.path_id);
            }
        }
        Ok(roots)
    }

    /// Every scene of the build: levels to `levels/`, the rest to
    /// `scenes/`. Also returns the asset GameObjects the scenes reference.
    fn scenes(&self) -> Result<(Vec<format::SceneEntry>, BTreeSet<ObjId>)> {
        let ggm = self.dir.file("globalgamemanagers")?;
        let bs = ggm.objects_of(141).next().context("no BuildSettings")?;
        let paths = classes::build_scenes(&self.dir.read(&ggm, bs.path_id)?);
        let results: Vec<Result<(format::SceneEntry, BTreeSet<ObjId>)>> = paths
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let name = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(path)
                    .trim_end_matches(".unity")
                    .to_owned();
                self.scene(i, &name, path)
                    .with_context(|| format!("scene {i} {name}"))
            })
            .collect();
        let mut entries = Vec::new();
        let mut refs = BTreeSet::new();
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok((e, r)) => {
                    entries.push(e);
                    refs.extend(r);
                }
                Err(e) => {
                    self.warn(format!("{e:#}"));
                    entries.push(format::SceneEntry {
                        build_index: i,
                        name: paths[i].clone(),
                        path: paths[i].clone(),
                        level: false,
                        file: None,
                    });
                }
            }
        }
        Ok((entries, refs))
    }

    fn scene(
        &self,
        i: usize,
        name: &str,
        path: &str,
    ) -> Result<(format::SceneEntry, BTreeSet<ObjId>)> {
        let f = self.dir.file(&format!("level{i}"))?;
        let roots = self.scene_roots(&f)?;
        let objects = self.ctx.flatten(&f, &roots)?;
        let refs = self.prefab_refs(&objects);
        let comp = |class: &str| {
            objects
                .iter()
                .enumerate()
                .find_map(|(i, o)| o.component(class).map(|c| (i, c)))
        };
        let session = comp("GameSessionHandler").map(|(_, c)| c.fields.clone());
        let level = session.is_some();
        let file = format!(
            "{}/{i:03}_{}.json",
            if level { "levels" } else { "scenes" },
            format::sanitize(name)
        );
        if let Some(session) = session {
            let fix_j = |j: &J| j["m_rawValue"].as_i64().unwrap_or(0);
            let fix2_j = |j: &J| [fix_j(&j["x"]), fix_j(&j["y"])];
            let with = |pred: &dyn Fn(&format::Object) -> bool| -> Vec<usize> {
                objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| pred(o))
                    .map(|(i, _)| i)
                    .collect()
            };
            let lv = format::Level {
                build_index: i,
                name: name.to_owned(),
                path: path.to_owned(),
                level_type: session["levelType"].as_i64().unwrap_or(0),
                team_spawns: session["teamSpawns"]
                    .as_array()
                    .map(|a| a.iter().map(fix2_j).collect())
                    .unwrap_or_default(),
                teammate_spawn_spacing: fix_j(&session["teammateSpawnSpacing"]),
                physics: comp("DetPhysics").map_or(J::Null, |(_, c)| c.fields.clone()),
                main_camera: comp("PlayerAverageCamera").map(|(i, _)| i),
                ability_spawner: comp("AbilitySpawner").map(|(i, _)| i),
                platforms: with(&|o| o.component("StickyRoundedRectangle").is_some()),
                shapes: with(&|o| o.shape.is_some()),
                session,
                objects,
            };
            write_json(&self.out.join(&file), &lv, false)?;
        } else {
            let sc = format::Scene {
                build_index: i,
                name: name.to_owned(),
                path: path.to_owned(),
                objects,
            };
            write_json(&self.out.join(&file), &sc, false)?;
        }
        Ok((
            format::SceneEntry {
                build_index: i,
                name: name.to_owned(),
                path: path.to_owned(),
                level,
                file: Some(file),
            },
            refs,
        ))
    }

    /// Every Material of every file.
    fn materials(&self, files: &Files) -> Result<BTreeMap<ObjId, format::Material>> {
        let mut shaders: BTreeMap<ObjId, String> = BTreeMap::new();
        let mut out = BTreeMap::new();
        for f in files {
            for o in f.objects_of(21) {
                let id = obj_id(f, o.path_id);
                let v = match self.dir.read(f, o.path_id) {
                    Ok(v) => v,
                    Err(e) => {
                        self.warn(format!("material {id}: {e}"));
                        continue;
                    }
                };
                let shader = match v.get("m_Shader").as_pptr() {
                    Some(p) if !p.is_null() => {
                        let sid = ref_id(f, p).unwrap_or_default();
                        if let Some(n) = shaders.get(&sid) {
                            n.clone()
                        } else {
                            let n = match self.dir.deref(f, p) {
                                Ok(Some((_, _, sv))) => {
                                    sv.path("m_ParsedForm.m_Name").str().to_owned()
                                }
                                _ => String::new(),
                            };
                            shaders.insert(sid, n.clone());
                            n
                        }
                    }
                    _ => String::new(),
                };
                let props = v.get("m_SavedProperties");
                let pairs = |name: &str| -> Vec<(String, &unity_assets::Value)> {
                    props
                        .get(name)
                        .as_array()
                        .iter()
                        .map(|p| (p.get("first").str().to_owned(), p.get("second")))
                        .collect()
                };
                out.insert(
                    id,
                    format::Material {
                        name: v.get("m_Name").str().to_owned(),
                        shader,
                        render_queue: v.get("m_CustomRenderQueue").i64() as i32,
                        keywords: v
                            .get("m_ValidKeywords")
                            .as_array()
                            .iter()
                            .map(|k| k.str().to_owned())
                            .collect(),
                        textures: pairs("m_TexEnvs")
                            .into_iter()
                            .map(|(k, t)| {
                                (
                                    k,
                                    format::TexEnv {
                                        texture: t
                                            .get("m_Texture")
                                            .as_pptr()
                                            .and_then(|p| ref_id(f, p)),
                                        scale: t.get("m_Scale").vec2(),
                                        offset: t.get("m_Offset").vec2(),
                                    },
                                )
                            })
                            .collect(),
                        floats: pairs("m_Floats")
                            .into_iter()
                            .map(|(k, x)| (k, x.f32()))
                            .collect(),
                        colors: pairs("m_Colors")
                            .into_iter()
                            .map(|(k, c)| (k, c.color()))
                            .collect(),
                    },
                );
            }
        }
        Ok(out)
    }

    /// Script MonoBehaviours of a class across files: (file, path id).
    fn scripts_of(&self, files: &Files, class: &str) -> Vec<(Arc<AssetFile>, i64)> {
        let mut out = Vec::new();
        for f in files {
            for o in f.objects_of(114) {
                if self.dir.script_of(f, o).is_some_and(|k| k.class == class) {
                    out.push((f.clone(), o.path_id));
                }
            }
        }
        out
    }

    /// A ScriptableObject asset by id.
    fn asset(&self, id: &str) -> Option<format::Asset> {
        let (file, pid) = id.rsplit_once(':')?;
        let f = self.dir.file(file).ok()?;
        let info = f.object(pid.parse().ok()?)?;
        let read = self.dir.read_object(&f, info).ok()?;
        Some(format::Asset {
            id: id.to_owned(),
            name: read.value.get("m_Name").str().to_owned(),
            script: read.script.map(|k| k.class).unwrap_or_default(),
            fields: script_fields(&f, &read.value),
        })
    }

    fn first_asset(&self, assets: &Files, class: &str) -> Option<format::Asset> {
        let (f, pid) = self.scripts_of(assets, class).into_iter().next()?;
        self.asset(&obj_id(&f, pid))
    }

    fn globals(&self, assets: &Files, scenes: &[format::SceneEntry]) -> Result<format::Globals> {
        let mut g = format::Globals {
            player_colors: self.first_asset(assets, "PlayerColors"),
            team_colors: self.first_asset(assets, "TeamColors"),
            beam_colors: self.first_asset(assets, "BeamColorList"),
            settings: self.first_asset(assets, "Settings"),
            ..Default::default()
        };
        for (f, pid) in self.scripts_of(assets, "NamedSpriteList") {
            let Some(a) = self.asset(&obj_id(&f, pid)) else {
                continue;
            };
            g.sprite_lists
                .insert(a.name.clone(), named_sprites(&a.fields));
            g.sprite_list_ids.insert(a.name.clone(), a.id);
        }
        // Singleton components: scan the scene files in build order.
        let wanted: HashSet<&str> = SINGLETONS.iter().copied().collect();
        for e in scenes {
            let Ok(f) = self.dir.file(&format!("level{}", e.build_index)) else {
                continue;
            };
            for o in f.objects_of(114) {
                let Some(k) = self.dir.script_of(&f, o) else {
                    continue;
                };
                if !wanted.contains(k.class.as_str()) || g.components.contains_key(&k.class) {
                    continue;
                }
                let v = self.dir.read(&f, o.path_id)?;
                g.components.insert(
                    k.class.clone(),
                    format::SceneComponent {
                        scene: e.build_index,
                        object: v
                            .get("m_GameObject")
                            .as_pptr()
                            .and_then(|p| ref_id(&f, p))
                            .unwrap_or_default(),
                        component: obj_id(&f, o.path_id),
                        fields: script_fields(&f, &v),
                    },
                );
            }
        }
        Ok(g)
    }

    fn abilities(&self, g: &format::Globals) -> Result<(format::Abilities, BTreeSet<ObjId>)> {
        // The lists SteamManager hands out (full / demo), else by name.
        let steam = g.components.get("SteamManager").map(|c| &c.fields);
        let list_id = |field: &str, name: &str| -> Option<ObjId> {
            steam
                .and_then(|s| s[field]["$ref"].as_str().map(str::to_owned))
                .or_else(|| g.sprite_list_ids.get(name).cloned())
        };
        let full_list = list_id("abilityIconsFull", "AbilityIcons");
        let demo_list = list_id("abilityIconsDemo", "AbilityIcons_demo");
        let mut refs = BTreeSet::new();
        let mut entries = |id: &Option<ObjId>| -> Vec<format::AbilityEntry> {
            let Some(a) = id.as_ref().and_then(|id| self.asset(id)) else {
                return vec![];
            };
            named_sprites(&a.fields)
                .into_iter()
                .enumerate()
                .map(|(index, s)| {
                    let mut e = format::AbilityEntry {
                        index,
                        name: s.name,
                        sprite: s.sprite,
                        prefab: s.object.clone(),
                        offensive: s.offensive,
                        ..Default::default()
                    };
                    if let Some(go) = &s.object {
                        refs.insert(go.clone());
                        if let Some(root) = self.object_root(go) {
                            for c in &root.components {
                                e.scripts.push(c.script.clone().unwrap_or(c.kind.clone()));
                                let s = c.script.as_deref();
                                if e.kind.is_none()
                                    && matches!(s, Some("Ability" | "InstantAbility"))
                                {
                                    e.kind = s.map(str::to_owned);
                                    e.component = Some(c.id.clone());
                                    e.fields = c.fields.clone();
                                }
                            }
                        }
                    }
                    e
                })
                .collect()
        };
        let full = entries(&full_list);
        let demo = entries(&demo_list);
        Ok((
            format::Abilities {
                full,
                demo,
                full_list,
                demo_list,
            },
            refs,
        ))
    }

    /// The flattened object `go` (just that object, without children).
    fn object_root(&self, go: &str) -> Option<format::Object> {
        let (file, pid) = go.rsplit_once(':')?;
        let f = self.dir.file(file).ok()?;
        let mut objs = self.ctx.flatten_shallow(&f, pid.parse().ok()?).ok()?;
        (!objs.is_empty()).then(|| objs.swap_remove(0))
    }

    fn player(&self, scenes: &[format::SceneEntry]) -> Result<format::Player> {
        let mut p = format::Player::default();
        let Some(first) = scenes.iter().find(|s| s.level && s.file.is_some()) else {
            self.warn("player: no level scene".into());
            return Ok(p);
        };
        let f = self.dir.file(&format!("level{}", first.build_index))?;
        let field_go = |v: &unity_assets::Value, k: &str| -> Option<(ObjId, ObjId)> {
            let c = ref_id(&f, v.get(k).as_pptr()?)?;
            Some((c.clone(), self.game_object_of(&c)?))
        };
        for (class, info) in self.scripts_of(std::slice::from_ref(&f), "GameSessionHandler") {
            let v = self.dir.read(&class, info)?;
            if let Some((c, go)) = field_go(&v, "PlayerPrefab") {
                p.controller = Some(c);
                p.prefab = Some(go);
            }
            p.input_updater = field_go(&v, "InputUpdaterPrefab").map(|x| x.1);
        }
        for (class, info) in self.scripts_of(std::slice::from_ref(&f), "AbilitySpawner") {
            let v = self.dir.read(&class, info)?;
            p.ability_pickup = field_go(&v, "pickupPrefab").map(|x| x.1);
        }
        if p.prefab.is_none() {
            self.warn("player: no GameSessionHandler.PlayerPrefab".into());
        }
        Ok(p)
    }

    /// GameObjects referenced from the components of `objects` but outside
    /// them, as prefab roots in asset files.
    fn prefab_refs(&self, objects: &[format::Object]) -> BTreeSet<ObjId> {
        let own: HashSet<&str> = objects
            .iter()
            .flat_map(|o| {
                std::iter::once(o.id.as_str()).chain(o.components.iter().map(|c| c.id.as_str()))
            })
            .collect();
        let mut ids = BTreeSet::new();
        for o in objects {
            for c in &o.components {
                crate::collect_refs(&c.fields, &mut ids);
            }
            if let Some(p) = &o.particles {
                for s in &p.sub_emitters {
                    ids.extend(s.emitter.iter().cloned());
                }
            }
        }
        let mut out = BTreeSet::new();
        for id in ids {
            if own.contains(id.as_str()) {
                continue;
            }
            if let Some(go) = self.game_object_of(&id)
                && !own.contains(go.as_str())
            {
                out.insert(go);
            }
        }
        out
    }

    /// The GameObject an ID denotes (itself, or a component's owner) in an
    /// asset file, if any.
    fn game_object_of(&self, id: &str) -> Option<ObjId> {
        let (file, pid) = id.rsplit_once(':')?;
        if file.starts_with("level") {
            return None;
        }
        let f = self.dir.file(file).ok()?;
        let pid: i64 = pid.parse().ok()?;
        let info = f.object(pid)?;
        match info.class_id {
            1 => Some(id.to_owned()),
            // Assets that are not part of a GameObject hierarchy.
            21 | 28 | 43 | 48 | 49 | 74 | 83 | 91 | 115 | 128 | 213 => None,
            _ => {
                let v = self.dir.read_object(&f, info).ok()?.value;
                let go = v.get("m_GameObject").as_pptr().filter(|p| !p.is_null())?;
                ref_id(&f, go)
            }
        }
    }

    /// The root GameObject of the prefab hierarchy containing `go`.
    fn prefab_root(&self, go: &str) -> Option<(Arc<AssetFile>, i64)> {
        let (file, pid) = go.rsplit_once(':')?;
        let f = self.dir.file(file).ok()?;
        let mut pid: i64 = pid.parse().ok()?;
        for _ in 0..256 {
            let g = classes::GameObject::from_value(&self.dir.read(&f, pid).ok()?);
            let t = g.components.first().filter(|c| c.file_id == 0)?;
            let tr = classes::Transform::from_value(&self.dir.read(&f, t.path_id).ok()?);
            if tr.father.is_null() || tr.father.file_id != 0 {
                return Some((f, pid));
            }
            let father =
                classes::Transform::from_value(&self.dir.read(&f, tr.father.path_id).ok()?);
            pid = father.game_object.path_id;
        }
        None
    }

    /// Exports the prefabs containing `start` and everything they
    /// reference, transitively; returns the index of what was written.
    fn prefab_closure(&self, start: BTreeSet<ObjId>) -> Result<format::PrefabIndex> {
        let mut index = format::PrefabIndex::default();
        let mut done: HashSet<ObjId> = HashSet::new();
        let roots = |ids: BTreeSet<ObjId>| -> BTreeSet<ObjId> {
            ids.par_iter()
                .filter_map(|id| self.prefab_root(id).map(|(f, pid)| obj_id(&f, pid)))
                .collect()
        };
        let mut frontier: Vec<ObjId> = roots(start).into_iter().collect();
        for _ in 0..PREFAB_DEPTH {
            frontier.retain(|id| done.insert(id.clone()));
            if frontier.is_empty() {
                break;
            }
            let results: Vec<(ObjId, Result<PrefabOut>)> = frontier
                .par_iter()
                .map(|id| (id.clone(), self.prefab(id)))
                .collect();
            let mut next = BTreeSet::new();
            for (id, r) in results {
                match r {
                    Ok((p, refs)) => {
                        let file = format::prefab_file(&id);
                        for o in &p.objects {
                            index.objects.insert(o.id.clone(), id.clone());
                            for c in &o.components {
                                index.objects.insert(c.id.clone(), id.clone());
                            }
                            if let Some(t) = &o.transform {
                                index.objects.insert(t.clone(), id.clone());
                            }
                        }
                        index.roots.insert(
                            id.clone(),
                            format::PrefabEntry {
                                name: p.name.clone(),
                                file,
                                objects: p.objects.len(),
                            },
                        );
                        next.extend(refs);
                    }
                    Err(e) => self.warn(format!("prefab {id}: {e:#}")),
                }
            }
            frontier = roots(next)
                .into_iter()
                .filter(|id| !done.contains(id))
                .collect();
        }
        if !frontier.is_empty() {
            self.warn(format!(
                "prefab references deeper than {PREFAB_DEPTH} levels not exported: {}",
                frontier.join(", ")
            ));
        }
        Ok(index)
    }

    fn prefab(&self, id: &str) -> Result<PrefabOut> {
        let (file, pid) = id.rsplit_once(':').context("bad id")?;
        let f = self.dir.file(file)?;
        let objects = self.ctx.flatten(&f, &[pid.parse()?])?;
        let refs = self.prefab_refs(&objects);
        let p = format::Prefab {
            id: id.to_owned(),
            name: objects[0].name.clone(),
            objects,
        };
        write_json(&self.out.join(format::prefab_file(id)), &p, false)?;
        Ok((p, refs))
    }

    /// Every ScriptableObject (MonoBehaviour without GameObject) of the
    /// asset files, except the font and localisation assets, which have
    /// their own files.
    fn script_assets(&self, assets: &Files) -> Result<BTreeMap<ObjId, format::Asset>> {
        let mut out = BTreeMap::new();
        for f in assets {
            for o in f.objects_of(114) {
                let read = self.dir.read_object(f, o)?;
                let v = &read.value;
                if v.get("m_GameObject")
                    .as_pptr()
                    .is_some_and(|p| !p.is_null())
                {
                    continue;
                }
                let script = read.script.as_ref().map(|k| k.class.clone());
                if matches!(
                    script.as_deref(),
                    Some("TMP_FontAsset" | "LocalizationTable")
                ) {
                    continue;
                }
                if !read.full_layout {
                    self.warn(format!(
                        "asset {}: no layout for script {script:?}",
                        obj_id(f, o.path_id)
                    ));
                }
                out.insert(
                    obj_id(f, o.path_id),
                    format::Asset {
                        id: obj_id(f, o.path_id),
                        name: v.get("m_Name").str().to_owned(),
                        script: read
                            .script
                            .as_ref()
                            .map(|k| k.full_name())
                            .unwrap_or_default(),
                        fields: script_fields(f, v),
                    },
                );
            }
        }
        Ok(out)
    }

    fn localization(&self, assets: &Files) -> Result<format::Localization> {
        let mut l = format::Localization::default();
        let Some(a) = self.first_asset(assets, "LocalizationTable") else {
            self.warn("localization: no LocalizationTable asset".into());
            return Ok(l);
        };
        l.id = Some(a.id.clone());
        if let J::Object(m) = &a.fields {
            for (k, v) in m {
                match v {
                    J::Array(items) => {
                        l.languages.push(k.clone());
                        l.strings.insert(
                            k.clone(),
                            items
                                .iter()
                                .map(|s| s.as_str().unwrap_or("").to_owned())
                                .collect(),
                        );
                    }
                    J::Object(r) if r.contains_key("$ref") => {
                        l.fonts
                            .insert(k.clone(), r["$ref"].as_str().map(str::to_owned));
                    }
                    J::Null => {
                        l.fonts.insert(k.clone(), None);
                    }
                    _ => {}
                }
            }
        }
        // Keep the languages in field order (serde_json's map is sorted).
        let order = [
            "en", "jp", "ko", "fr", "it", "de", "es", "ptbr", "ru", "tr", "se", "zhcn", "zhtw",
            "pl",
        ];
        l.languages
            .sort_by_key(|k| order.iter().position(|o| o == k).unwrap_or(order.len()));
        Ok(l)
    }

    fn fonts(&self, assets: &Files) -> Result<Vec<format::Font>> {
        let mut fonts = Vec::new();
        for (f, pid) in self.scripts_of(assets, "TMP_FontAsset") {
            let v = self.dir.read(&f, pid)?;
            let atlases = v
                .get("m_AtlasTextures")
                .as_array()
                .iter()
                .filter_map(|p| p.as_pptr().and_then(|p| ref_id(&f, p)))
                .collect();
            let name = v.get("m_Name").str().to_owned();
            let mut fields = script_fields(&f, &v);
            let (info, suspicious) = crate::fonts::font_info(&mut fields);
            if !suspicious.is_empty() {
                self.warn(format!(
                    "font {name}: glyph rects outside the atlas for {}",
                    suspicious.join(", ")
                ));
            }
            fonts.push(format::Font {
                id: obj_id(&f, pid),
                name,
                atlases,
                source_font: fields["m_SourceFontFile"]["$ref"]
                    .as_str()
                    .map(str::to_owned),
                fields,
                info: Some(info),
            });
        }
        Ok(fonts)
    }

    /// Unity `Font` assets with embedded data, written to `fonts/`.
    fn font_files(&self, assets: &Files) -> Result<BTreeMap<ObjId, format::FontFile>> {
        let mut out = BTreeMap::new();
        let mut used = HashSet::new();
        for f in assets {
            for o in f.objects_of(128) {
                let id = obj_id(f, o.path_id);
                let v = self.dir.read(f, o.path_id)?;
                let Some(data) = v.get("m_FontData").as_bytes().filter(|d| !d.is_empty()) else {
                    continue;
                };
                let name = v.get("m_Name").str().to_owned();
                let ext = if data.starts_with(b"OTTO") {
                    "otf"
                } else {
                    "ttf"
                };
                let base = format::sanitize(&name);
                let mut stem = base.clone();
                let mut n = 1;
                while !used.insert(stem.to_lowercase()) {
                    n += 1;
                    stem = format!("{base}_{n}");
                }
                let file = format!("fonts/{stem}.{ext}");
                let hash = fnv64(&[data]);
                let dest = self.out.join(&file);
                let reused = self.old.as_ref().is_some_and(|o| {
                    let old = o.font_files.get(&id);
                    old.is_some_and(|x| x.file == file)
                        && o.reuse(
                            &file,
                            old.and_then(|x| x.source_hash.as_deref()),
                            &hash,
                            &dest,
                        )
                });
                if !reused {
                    fs::write(&dest, data)?;
                }
                out.insert(
                    id,
                    format::FontFile {
                        name,
                        file,
                        family_names: v
                            .get("m_FontNames")
                            .as_array()
                            .iter()
                            .map(|s| s.str().to_owned())
                            .collect(),
                        font_size: v.get("m_FontSize").f32(),
                        ascent: v.get("m_Ascent").f32(),
                        descent: v.get("m_Descent").f32(),
                        line_spacing: v.get("m_LineSpacing").f32(),
                        fallbacks: v
                            .get("m_FallbackFonts")
                            .as_array()
                            .iter()
                            .filter_map(|p| p.as_pptr().and_then(|p| ref_id(f, p)))
                            .collect(),
                        source_hash: Some(hash),
                    },
                );
            }
        }
        Ok(out)
    }

    fn settings(&self, ggm: &AssetFile) -> Result<format::ProjectSettings> {
        let mut s = format::ProjectSettings::default();
        if let Some(o) = ggm.objects_of(5).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            s.fixed_timestep = v.get("Fixed Timestep").f32();
            s.time = to_json(ggm, &v);
        }
        if let Some(o) = ggm.objects_of(19).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            s.gravity = v.get("m_Gravity").vec2();
            s.physics2d = to_json(ggm, &v);
        }
        if let Some(o) = ggm.objects_of(129).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            let mut m = serde_json::Map::new();
            for k in [
                "productName",
                "companyName",
                "bundleVersion",
                "defaultScreenWidth",
                "defaultScreenHeight",
            ] {
                m.insert(k.into(), to_json(ggm, v.get(k)));
            }
            s.player = J::Object(m);
        }
        Ok(s)
    }
}

/// `NamedSpriteList.sprites` from raw fields.
fn named_sprites(fields: &J) -> Vec<format::NamedSprite> {
    let r = |j: &J| j["$ref"].as_str().map(str::to_owned);
    fields["sprites"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| format::NamedSprite {
            name: s["name"].as_str().unwrap_or("").to_owned(),
            sprite: r(&s["sprite"]),
            object: r(&s["associatedGameObject"]),
            offensive: s["isOffensiveAbility"].as_i64().unwrap_or(0) != 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_detects_changes() {
        assert_eq!(fnv64(&[b"abc"]), fnv64(&[b"abc"]));
        assert_ne!(fnv64(&[b"abc"]), fnv64(&[b"abd"]));
        assert_ne!(fnv64(&[b"ab", b"c"]), fnv64(&[b"a", b"bc"]));
        assert_ne!(fnv64(&[&[0u8; 16]]), fnv64(&[&[0u8; 17]]));
    }

    #[test]
    fn named_sprite_list() {
        let j = serde_json::json!({"sprites": [
            {"name": "Dash", "sprite": {"$ref": "s:1"}, "associatedGameObject": {"$ref": "s:2"}, "isOffensiveAbility": 0},
            {"name": "Rock", "sprite": null, "associatedGameObject": null, "isOffensiveAbility": 1}
        ]});
        let l = named_sprites(&j);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].sprite.as_deref(), Some("s:1"));
        assert_eq!(l[0].object.as_deref(), Some("s:2"));
        assert!(!l[0].offensive && l[1].offensive && l[1].sprite.is_none());
    }
}
