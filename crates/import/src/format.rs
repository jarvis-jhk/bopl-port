//! The on-disk import format (see `docs/formats/import.md`). These types are
//! written by the importer and read back by [`crate::GameData`].
//!
//! Every imported Unity object is identified by an [`ObjId`] string
//! `"<file>:<path id>"`, e.g. `"sharedassets0.assets:2240"`. In raw field
//! dumps (`fields`), object references are written as `{"$ref": "<id>"}` and
//! null references as `null`. Fixed-point values (the game's `Fix`, Q32.32
//! in an `i64`) stay raw: `{"m_rawValue": <i64>}` in `fields`, plain `i64`
//! ([`FixRaw`]) in the typed parts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Version of the format; bump on incompatible changes (re-import).
pub const FORMAT_VERSION: u32 = 1;

/// Additive revision within [`FORMAT_VERSION`]: new optional fields and
/// files. An import with an older revision still loads (the new data is
/// empty), and [`crate::import`] re-imports it.
///
/// - 0: the first release of version 1.
pub const FORMAT_REVISION: u32 = 0;

/// Global object identifier `"<file>:<path id>"`.
pub type ObjId = String;

/// Raw field values of a component (Unity field names), references as `{"$ref": id}`.
pub type Fields = serde_json::Value;

/// A Q32.32 fixed-point number as its raw bits (`value = raw / 2^32`).
pub type FixRaw = i64;

/// `index.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Index {
    /// [`FORMAT_VERSION`] at import time.
    pub format_version: u32,
    /// [`FORMAT_REVISION`] at import time (0 when absent).
    #[serde(default)]
    pub format_revision: u32,
    /// Unity version of the source data.
    pub unity_version: String,
    /// Fingerprint of the source files (names, sizes and modification times).
    pub source_fingerprint: String,
    /// Scenes in build order.
    pub scenes: Vec<SceneEntry>,
    /// Sorting layers in draw order.
    pub sorting_layers: Vec<SortingLayer>,
    /// Physics/render layer names by index (32).
    pub layers: Vec<String>,
    /// Tag names (index 0 = "Untagged", then the built-in and custom tags).
    pub tags: Vec<String>,
    /// Fixed timestep and other project settings of interest.
    pub settings: ProjectSettings,
    /// GameObject ID of the player prefab (see `player.json`).
    pub player_prefab: Option<ObjId>,
    /// Object counts per output kind.
    pub counts: BTreeMap<String, usize>,
}

/// A scene from the build settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneEntry {
    /// Build index (`levelN`).
    pub build_index: usize,
    /// Scene name, e.g. `Level1`, `esnow 3`.
    pub name: String,
    /// Asset path in the original project.
    pub path: String,
    /// Whether the scene is a playable level (has a `GameSessionHandler`).
    pub level: bool,
    /// Scene file relative to the import dir: `levels/NNN_<name>.json`
    /// ([`Level`]) for levels, `scenes/NNN_<name>.json` ([`Scene`]) for the
    /// others.
    pub file: Option<String>,
}

/// A sorting layer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SortingLayer {
    /// Layer name.
    pub name: String,
    /// The ID renderers refer to.
    pub id: u32,
}

/// Project settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectSettings {
    /// `Time.fixedDeltaTime` (the game steps its own `Fix` simulation in
    /// `Updater`; see the specs).
    pub fixed_timestep: f32,
    /// `Physics2D.gravity` (unused by the game's own physics).
    pub gravity: [f32; 2],
    /// Raw `TimeManager` fields.
    pub time: Fields,
    /// Raw `Physics2DSettings` fields.
    pub physics2d: Fields,
    /// Raw `PlayerSettings` fields of interest: `defaultScreenWidth`,
    /// `defaultScreenHeight`, `productName`, `companyName`, `bundleVersion`.
    pub player: Fields,
}

/// One entry of `textures.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Texture {
    /// Asset name.
    pub name: String,
    /// PNG file relative to the import dir (RGBA8, top row first).
    pub png: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Original Unity format name.
    pub format: String,
    /// 0 point, 1 bilinear, 2 trilinear.
    pub filter_mode: i32,
    /// 0 repeat, 1 clamp.
    pub wrap_mode: i32,
    /// Hash of the source data (dimensions, format, first mip level); the
    /// importer reuses the PNG of an unchanged texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// One entry of `sprites.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sprite {
    /// Asset name.
    pub name: String,
    /// Texture (atlas page when packed).
    pub texture: Option<ObjId>,
    /// Sprite rectangle in its source image, pixels, origin bottom-left.
    pub rect: [f32; 4],
    /// Rectangle inside `texture`, pixels, origin **top-left** (PNG space).
    pub texture_rect: [f32; 4],
    /// Offset of `texture_rect` inside `rect` (trimmed packing), pixels.
    pub texture_rect_offset: [f32; 2],
    /// Pivot, normalised in `rect` (0,0 = bottom-left).
    pub pivot: [f32; 2],
    /// Pixels per world unit.
    pub pixels_per_unit: f32,
    /// 9-slice border `[left, bottom, right, top]`, pixels.
    pub border: [f32; 4],
    /// Packed into an atlas.
    pub packed: bool,
    /// Packing rotation (0 none, 1 flip H, 2 flip V, 3 rot 180, 4 rot 90).
    pub packing_rotation: u32,
    /// Physics shape outlines in world units relative to the pivot.
    pub physics_shape: Vec<Vec<[f32; 2]>>,
}

/// One entry of `audio.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioClip {
    /// Asset name.
    pub name: String,
    /// Ogg Vorbis file relative to the import dir.
    pub file: String,
    /// Channel count.
    pub channels: u32,
    /// Sample rate.
    pub frequency: u32,
    /// Length in seconds.
    pub length: f32,
    /// Samples per channel.
    pub samples: u64,
    /// Hash of the source FSB5 bank (see [`Texture::source_hash`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// Translation, rotation, scale.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Trs {
    /// Position.
    pub position: [f32; 3],
    /// Rotation quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Scale (for world transforms: lossy scale).
    pub scale: [f32; 3],
}

/// World transform of an object.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldTransform {
    /// Position, rotation, lossy scale.
    #[serde(flatten)]
    pub trs: Trs,
    /// Rotation about Z in degrees (counter-clockwise).
    pub rotation_z: f32,
    /// 2D affine part of the world matrix `[a, b, c, d, tx, ty]`:
    /// `x' = a x + c y + tx`, `y' = b x + d y + ty` (exact for skewed hierarchies).
    pub matrix: [f32; 6],
}

/// `RectTransform` layout fields.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RectTransform {
    /// Anchor minimum (normalised in the parent rect).
    pub anchor_min: [f32; 2],
    /// Anchor maximum.
    pub anchor_max: [f32; 2],
    /// Pivot position relative to the anchors.
    pub anchored_position: [f32; 2],
    /// Size relative to the anchors (the size when both anchors coincide).
    pub size_delta: [f32; 2],
    /// Pivot (normalised in the own rect).
    pub pivot: [f32; 2],
}

/// A sprite renderer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpriteRenderer {
    /// Component enabled.
    pub enabled: bool,
    /// Sprite drawn (may be missing).
    pub sprite: Option<ObjId>,
    /// Tint.
    pub color: [f32; 4],
    /// Mirror X.
    pub flip_x: bool,
    /// Mirror Y.
    pub flip_y: bool,
    /// 0 simple, 1 sliced, 2 tiled.
    pub draw_mode: i32,
    /// Size for sliced/tiled modes, world units.
    pub size: [f32; 2],
    /// Sorting layer name.
    pub sorting_layer: String,
    /// Index of the sorting layer in draw order.
    pub sorting_layer_index: usize,
    /// Order in layer.
    pub sorting_order: i32,
    /// 0 none, 1 visible inside mask, 2 visible outside mask.
    pub mask_interaction: i32,
    /// Material names.
    pub materials: Vec<String>,
    /// Material IDs (`materials.json`), index-aligned with `materials`.
    pub material_ids: Vec<ObjId>,
}

/// A component other than Transform, in raw form.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Component {
    /// Component ID.
    pub id: ObjId,
    /// Unity class name (`MonoBehaviour` for scripts).
    #[serde(rename = "type")]
    pub kind: String,
    /// Script class (namespace-qualified) for MonoBehaviours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// `m_Enabled` of Behaviours (MonoBehaviours, Animator, ...), Renderers
    /// and Colliders; None for components without the flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Field values (omitted for bulky engine classes like ParticleSystem).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub fields: Fields,
}

/// A texture slot of a [`Material`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TexEnv {
    /// Texture, if assigned.
    pub texture: Option<ObjId>,
    /// Tiling.
    pub scale: [f32; 2],
    /// Offset.
    pub offset: [f32; 2],
}

/// One entry of `materials.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Material {
    /// Asset name.
    pub name: String,
    /// Shader name (`m_ParsedForm.m_Name`), empty if unresolved.
    pub shader: String,
    /// `m_CustomRenderQueue` (-1 = shader default).
    pub render_queue: i32,
    /// Enabled shader keywords.
    pub keywords: Vec<String>,
    /// Texture properties.
    pub textures: BTreeMap<String, TexEnv>,
    /// Float properties (non-finite values are written as `null` and read
    /// back as infinity).
    #[serde(deserialize_with = "lossy::floats")]
    pub floats: BTreeMap<String, f32>,
    /// Colour properties (as authored, sRGB for colours; `null` components
    /// are infinity).
    #[serde(deserialize_with = "lossy::colors")]
    pub colors: BTreeMap<String, [f32; 4]>,
}

/// Deserializers that accept `null` for non-finite floats.
mod lossy {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer};

    fn f(v: Option<f32>) -> f32 {
        v.unwrap_or(f32::INFINITY)
    }

    pub fn floats<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, f32>, D::Error> {
        let m = BTreeMap::<String, Option<f32>>::deserialize(d)?;
        Ok(m.into_iter().map(|(k, v)| (k, f(v))).collect())
    }

    pub fn colors<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, [f32; 4]>, D::Error> {
        let m = BTreeMap::<String, [Option<f32>; 4]>::deserialize(d)?;
        Ok(m.into_iter().map(|(k, v)| (k, v.map(f))).collect())
    }
}

/// One entry of `fonts.json` (TextMeshPro font assets).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Font {
    /// MonoBehaviour ID.
    pub id: ObjId,
    /// Asset name.
    pub name: String,
    /// SDF atlas textures.
    pub atlases: Vec<ObjId>,
    /// All fields (face info, glyph and character tables, kerning, ...).
    /// Glyph rects outside the atlas (whitespace glyphs of fonts upgraded
    /// from TMP's legacy glyph list, see [`FontInfo`]) are replaced by an
    /// empty rect at 0,0.
    pub fields: Fields,
    /// Typed metrics, glyph, character and kerning tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<FontInfo>,
    /// The source font (`m_SourceFontFile`, see `font_files.json`): dynamic
    /// assets (`m_AtlasPopulationMode` 1, empty atlas) rasterise their
    /// glyphs from it at run time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_font: Option<ObjId>,
}

/// One entry of `font_files.json`: a Unity `Font` asset with embedded font
/// data, written to `fonts/<name>.ttf` (or `.otf`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontFile {
    /// Asset name.
    pub name: String,
    /// Font file relative to the import dir.
    pub file: String,
    /// `m_FontNames` (family names).
    pub family_names: Vec<String>,
    /// `m_FontSize`, `m_Ascent`, `m_Descent`, `m_LineSpacing` (legacy text
    /// metrics at `font_size`).
    pub font_size: f32,
    /// See [`FontFile::font_size`].
    pub ascent: f32,
    /// See [`FontFile::font_size`].
    pub descent: f32,
    /// See [`FontFile::font_size`].
    pub line_spacing: f32,
    /// `m_FallbackFonts`.
    pub fallbacks: Vec<ObjId>,
    /// Hash of the font data (see [`Texture::source_hash`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// Typed TextMeshPro font metrics. Units are atlas pixels at
/// `point_size`; scale by `size / point_size` for a font size.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontInfo {
    /// Atlas size in pixels.
    pub atlas_width: u32,
    /// See [`FontInfo::atlas_width`].
    pub atlas_height: u32,
    /// SDF spread around each glyph rect, pixels (`m_AtlasPadding`).
    pub padding: f32,
    /// `m_AtlasRenderMode` (GlyphRenderMode, e.g. 4165 SDFAA).
    pub render_mode: i64,
    /// Sampling point size of the atlas (`m_FaceInfo.m_PointSize`).
    pub point_size: f32,
    /// `m_FaceInfo.m_Scale`.
    pub scale: f32,
    /// Line metrics (`m_FaceInfo`): line height, ascent (> 0), descent
    /// (< 0), baseline, cap and mean line, underline offset/thickness, tab
    /// width.
    pub line_height: f32,
    /// See [`FontInfo::line_height`].
    pub ascent: f32,
    /// See [`FontInfo::line_height`].
    pub descent: f32,
    /// See [`FontInfo::line_height`].
    pub baseline: f32,
    /// See [`FontInfo::line_height`].
    pub cap_line: f32,
    /// See [`FontInfo::line_height`].
    pub mean_line: f32,
    /// See [`FontInfo::line_height`].
    pub underline_offset: f32,
    /// See [`FontInfo::line_height`].
    pub underline_thickness: f32,
    /// See [`FontInfo::line_height`].
    pub tab_width: f32,
    /// Bold style weight (`boldStyle`) and extra bold spacing (`boldSpacing`).
    pub bold_style: f32,
    /// See [`FontInfo::bold_style`].
    pub bold_spacing: f32,
    /// Glyphs by glyph index order of `m_GlyphTable`.
    pub glyphs: Vec<FontGlyph>,
    /// Character map: unicode code point -> glyph index.
    pub characters: BTreeMap<u32, u32>,
    /// Pair adjustments (kerning) by glyph index.
    pub kerning: Vec<FontKerning>,
}

/// One glyph of a [`FontInfo`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontGlyph {
    /// Glyph index (`m_Index`, the target of [`FontInfo::characters`]).
    pub index: u32,
    /// Atlas texture index (into [`Font::atlases`]).
    pub atlas: u32,
    /// Rect in the atlas, pixels, origin **bottom-left** as in TMP
    /// (`[x, y, w, h]`, padding not included). Always inside the atlas;
    /// `[0, 0, 0, 0]` for glyphs without pixels (whitespace).
    pub rect: [f32; 4],
    /// The same rect in PNG space (origin top-left).
    pub rect_png: [f32; 4],
    /// Glyph metrics: size, bearing from the pen position (x right, y up
    /// from the baseline to the glyph's top) and advance.
    pub width: f32,
    /// See [`FontGlyph::width`].
    pub height: f32,
    /// See [`FontGlyph::width`].
    pub bearing_x: f32,
    /// See [`FontGlyph::width`].
    pub bearing_y: f32,
    /// See [`FontGlyph::width`].
    pub advance: f32,
    /// Glyph scale (`m_Scale`).
    pub scale: f32,
}

/// A glyph pair adjustment (`m_FontFeatureTable.m_GlyphPairAdjustmentRecords`):
/// value records `[x placement, y placement, x advance, y advance]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontKerning {
    /// First glyph index.
    pub first: u32,
    /// Second glyph index.
    pub second: u32,
    /// Adjustment of the first glyph.
    pub first_adjust: [f32; 4],
    /// Adjustment of the second glyph.
    pub second_adjust: [f32; 4],
}

impl FontInfo {
    /// The glyph of a character, if mapped.
    pub fn glyph_for(&self, c: char) -> Option<&FontGlyph> {
        let gi = *self.characters.get(&(c as u32))?;
        self.glyphs.iter().find(|g| g.index == gi)
    }

    /// Kerning: the first glyph's x advance adjustment for a pair.
    pub fn kerning_x(&self, first: u32, second: u32) -> f32 {
        self.kerning
            .iter()
            .find(|k| k.first == first && k.second == second)
            .map_or(0.0, |k| k.first_adjust[2])
    }
}

/// `animations.json`: Mecanim animation clips and controllers
/// (`Animator.m_Controller` in scenes and prefabs).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Animations {
    /// `AnimationClip` assets by id.
    pub clips: BTreeMap<ObjId, AnimClip>,
    /// `AnimatorController` assets by id.
    pub controllers: BTreeMap<ObjId, AnimController>,
}

/// An animation clip as piecewise cubic curves.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimClip {
    /// Asset name.
    pub name: String,
    /// Start and stop time in seconds (the clip length is `stop - start`).
    pub start: f32,
    /// See [`AnimClip::start`].
    pub stop: f32,
    /// Authoring sample rate.
    pub sample_rate: f32,
    /// Loops (`m_LoopTime`).
    pub looping: bool,
    /// Animated values.
    pub curves: Vec<AnimCurve>,
    /// Objects of object-reference curves (sprite swaps): a curve's value
    /// (rounded down) indexes this list.
    pub pptr: Vec<Option<ObjId>>,
    /// Raw animation events (`m_Events`).
    pub events: Fields,
}

/// One animated value.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimCurve {
    /// CRC32 of the target's transform path relative to the Animator's
    /// object (names joined by `/`, `""` = the Animator's object = 0); see
    /// [`path_hash`].
    pub path: u32,
    /// Unity class id of the animated component (4 Transform, 224
    /// RectTransform, 1 GameObject, 114 MonoBehaviour, 212 SpriteRenderer,
    /// 225 CanvasGroup, ...).
    pub class_id: i32,
    /// Script class (`Namespace.Class`) for MonoBehaviour targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// Property: for Transforms `position.x|y|z`, `rotation.x|y|z|w`
    /// (quaternion), `euler.x|y|z` (degrees), `scale.x|y|z`; otherwise the
    /// serialized property path (`m_Color.a`, `m_IsActive`,
    /// `m_AnchoredPosition.x`, `m_Sprite`, ...) or `#<crc32>` if unresolved.
    pub property: String,
    /// Object-reference curve (value indexes [`AnimClip::pptr`]).
    #[serde(default)]
    pub pptr: bool,
    /// `[t0, a, b, c, d]` pieces sorted by `t0`: at time `t` use the last
    /// piece with `t0 <= t` and evaluate `((a*dt + b)*dt + c)*dt + d`,
    /// `dt = t - t0` (before the first piece: its `d`).
    pub segments: Vec<[f32; 5]>,
}

impl AnimCurve {
    /// Value at clip time `t`.
    pub fn eval(&self, t: f32) -> f32 {
        let i = self.segments.partition_point(|s| s[0] <= t);
        let Some(s) = self.segments.get(i.saturating_sub(1)) else {
            return 0.0;
        };
        let dt = (t - s[0]).max(0.0);
        ((s[1] * dt + s[2]) * dt + s[3]) * dt + s[4]
    }
}

/// Unity's hash of a transform path / property name (CRC32, zlib
/// polynomial).
pub fn path_hash(s: &str) -> u32 {
    let mut c = !0u32;
    for &b in s.as_bytes() {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// An `AnimatorController`: its layers' state machines with transitions
/// and parameters .
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimController {
    /// Asset name.
    pub name: String,
    /// Every clip the controller uses.
    pub clips: Vec<ObjId>,
    /// Layers in order.
    pub layers: Vec<AnimLayer>,
    /// Parameters with their default values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<AnimParameter>,
}

/// An Animator parameter.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimParameter {
    /// Name (or `#<hash>`).
    pub name: String,
    /// `float`, `int`, `bool` or `trigger`.
    pub kind: String,
    /// Default value (bools and triggers as 0/1).
    pub default: f32,
}

/// A state transition. Times are normalised to the source
/// state's length unless `fixed_duration` (then `duration` is seconds).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimTransition {
    /// Destination state index in the layer, None for an exit or
    /// sub-state-machine selector (see `destination_raw`).
    pub destination: Option<usize>,
    /// Raw `m_DestinationState` (>= 30000: selector states).
    pub destination_raw: u32,
    /// Cross-fade duration.
    pub duration: f32,
    /// Normalised start time in the destination state.
    pub offset: f32,
    /// Normalised exit time of the source state.
    pub exit_time: f32,
    /// Only fires once the exit time has passed (and conditions hold).
    pub has_exit_time: bool,
    /// `duration` is in seconds (else normalised).
    pub fixed_duration: bool,
    /// 0 none, 1 source, 2 destination, 3 source then destination,
    /// 4 destination then source.
    pub interruption_source: i32,
    /// `m_OrderedInterruption`.
    pub ordered_interruption: bool,
    /// Any-state transitions only: may re-enter the current state.
    pub can_transition_to_self: bool,
    /// All must hold.
    pub conditions: Vec<AnimCondition>,
}

/// A transition condition.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimCondition {
    /// `if`, `if_not` (bools/triggers), `greater`, `less`, `equals`,
    /// `not_equal` (numbers), `exit_time`, or `#<n>`.
    pub mode: String,
    /// Parameter name (or `#<hash>`).
    pub parameter: String,
    /// Threshold for numeric modes.
    pub threshold: f32,
}

/// A controller layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimLayer {
    /// Layer name.
    pub name: String,
    /// Weight (layer 0 always counts as 1).
    pub default_weight: f32,
    /// Index of the state entered first.
    pub default_state: usize,
    /// States of the layer's state machine.
    pub states: Vec<AnimState>,
    /// Transitions from any state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_state_transitions: Vec<AnimTransition>,
}

/// A state of a layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimState {
    /// Full path name (`Base Layer.Anim_Bg1`) or `#<hash>`.
    pub name: String,
    /// Playback speed.
    pub speed: f32,
    /// Loops.
    pub looping: bool,
    /// The clip played (first leaf of its blend tree).
    pub clip: Option<ObjId>,
    /// Number of outgoing transitions (see `outgoing`).
    pub transitions: usize,
    /// Outgoing transitions in priority order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outgoing: Vec<AnimTransition>,
    /// Normalised cycle offset.
    #[serde(default)]
    pub cycle_offset: f32,
    /// Parameter multiplying `speed`, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_parameter: Option<String>,
}

/// File name of a prefab, derived from its ID.
pub fn prefab_file(id: &str) -> String {
    format!("prefabs/{}.json", sanitize(id))
}

/// Replaces characters that are awkward in file names.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The game's own transform (`FixTransform` script): the authoritative
/// simulation position and rotation, raw Q32.32.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FixTransform {
    /// Component ID.
    pub component: ObjId,
    /// `_position` (world units).
    pub position: [FixRaw; 2],
    /// `_rotation` (radians, counter-clockwise).
    pub rotation: FixRaw,
    /// `_up` unit vector.
    pub up: [FixRaw; 2],
    /// `_right` unit vector.
    pub right: [FixRaw; 2],
    /// `offset`.
    pub offset: [FixRaw; 2],
}

/// Geometry of a [`DetShape`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetShapeKind {
    /// `DPhysicsBox`.
    Box {
        /// `startExtents` (half size, world units).
        extents: [FixRaw; 2],
        /// `minimumExtents`.
        minimum_extents: FixRaw,
    },
    /// `DPhysicsCircle`.
    Circle {
        /// `startRadius`.
        radius: FixRaw,
    },
    /// `DPhysicsRoundedRect`: a box of half size `extents` grown by `radius`.
    RoundedRect {
        /// `startExtents` (inner half size).
        extents: [FixRaw; 2],
        /// `startRadius`.
        radius: FixRaw,
    },
}

/// A shape of the game's deterministic physics (`DetPhysics`), raw Q32.32.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DetShape {
    /// Component ID.
    pub component: ObjId,
    /// `m_Enabled`.
    pub enabled: bool,
    /// Shape and size.
    #[serde(flatten)]
    pub kind: DetShapeKind,
    /// `hierarchyNumber` (the `Updater`'s ordering key).
    pub hierarchy_number: i64,
    /// `MinScale`.
    pub min_scale: FixRaw,
    /// `MaxScale`.
    pub max_scale: FixRaw,
    /// `manuallyCallInit`.
    pub manually_call_init: bool,
}

/// A GameObject in a flattened hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Object {
    /// GameObject ID.
    pub id: ObjId,
    /// Name.
    pub name: String,
    /// Index of the parent in the same list.
    pub parent: Option<usize>,
    /// Indices of the children in order.
    pub children: Vec<usize>,
    /// `activeSelf`.
    pub active: bool,
    /// Active including all ancestors.
    pub active_in_hierarchy: bool,
    /// Layer index.
    pub layer: u32,
    /// Id of the object's `Transform`/`RectTransform` component: fields
    /// referencing a Transform (`{"$ref": id}`) point to this object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ObjId>,
    /// Tag name.
    pub tag: String,
    /// Local transform (Unity's float `Transform`).
    pub local: Trs,
    /// World transform (relative to the prefab root's parent for prefabs).
    pub world: WorldTransform,
    /// `RectTransform` layout (UI objects), absent for plain Transforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<RectTransform>,
    /// Sprite renderer, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<SpriteRenderer>,
    /// The game's fixed-point transform, if the object has a `FixTransform`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixTransform>,
    /// The object's `DPhysicsBox` / `DPhysicsCircle` / `DPhysicsRoundedRect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<DetShape>,
    /// Typed particle system and renderer of this object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub particles: Option<Box<crate::particles::ParticleSystem>>,
    /// Every component except the Transform, raw (including the ones
    /// extracted into the typed fields above, except `SpriteRenderer`,
    /// `ParticleSystem` and `ParticleSystemRenderer`).
    pub components: Vec<Component>,
}

impl Object {
    /// The first component with the given script class (or engine class).
    pub fn component(&self, class: &str) -> Option<&Component> {
        self.components
            .iter()
            .find(|c| c.script.as_deref().unwrap_or(&c.kind) == class)
    }
}

/// `scenes/NNN_<name>.json`: a non-level scene (menus, character select,
/// winner triangle, tutorial).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Scene {
    /// Build index.
    pub build_index: usize,
    /// Scene name.
    pub name: String,
    /// Asset path.
    pub path: String,
    /// Every GameObject of the scene, parents before children, roots in
    /// file order.
    pub objects: Vec<Object>,
}

/// `levels/NNN_<name>.json`: a playable level.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Level {
    /// Build index (6..=129).
    pub build_index: usize,
    /// Scene name.
    pub name: String,
    /// Asset path.
    pub path: String,
    /// `GameSessionHandler.levelType`: 0 grass, 1 snow, 2 space.
    pub level_type: i64,
    /// `GameSessionHandler.teamSpawns` (world units, raw Q32.32).
    pub team_spawns: Vec<[FixRaw; 2]>,
    /// `GameSessionHandler.teammateSpawnSpacing`.
    pub teammate_spawn_spacing: FixRaw,
    /// Raw `GameSessionHandler` fields.
    pub session: Fields,
    /// Raw `DetPhysics` fields (gravity, rope and beam constants).
    pub physics: Fields,
    /// Index of the object holding `PlayerAverageCamera` (the main camera).
    pub main_camera: Option<usize>,
    /// Index of the `AbilitySpawner` object.
    pub ability_spawner: Option<usize>,
    /// Indices of the platform objects (`StickyRoundedRectangle`), in
    /// object order.
    pub platforms: Vec<usize>,
    /// Indices of every object with a [`DetShape`], in object order.
    pub shapes: Vec<usize>,
    /// Every GameObject of the scene, parents before children, roots in
    /// file order.
    pub objects: Vec<Object>,
}

/// `prefabs/<id>.json`: a prefab hierarchy (root first).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Prefab {
    /// Root GameObject ID.
    pub id: ObjId,
    /// Root name.
    pub name: String,
    /// Objects, root at index 0; world transforms are relative to the root's parent.
    pub objects: Vec<Object>,
}

/// `prefab_index.json`: what was written to `prefabs/`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PrefabIndex {
    /// Prefab root GameObject -> entry.
    pub roots: BTreeMap<ObjId, PrefabEntry>,
    /// Every GameObject, component and Transform inside an exported prefab
    /// -> its prefab root (resolves any `$ref` into a prefab).
    pub objects: BTreeMap<ObjId, ObjId>,
}

/// One exported prefab.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PrefabEntry {
    /// Root name.
    pub name: String,
    /// File relative to the import dir.
    pub file: String,
    /// Number of GameObjects.
    pub objects: usize,
}

/// `player.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Player {
    /// Player prefab root GameObject (`GameSessionHandler.PlayerPrefab`'s
    /// object), under `prefabs/`.
    pub prefab: Option<ObjId>,
    /// Its `SlimeController` component.
    pub controller: Option<ObjId>,
    /// `GameSessionHandler.InputUpdaterPrefab`'s GameObject.
    pub input_updater: Option<ObjId>,
    /// `AbilitySpawner.pickupPrefab`'s GameObject (the ability pickup).
    pub ability_pickup: Option<ObjId>,
}

/// `abilities.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Abilities {
    /// The full ability list (`AbilityIcons`, the order is the ability
    /// index the game uses everywhere).
    pub full: Vec<AbilityEntry>,
    /// The demo list (`AbilityIcons_demo`).
    pub demo: Vec<AbilityEntry>,
    /// IDs of the two `NamedSpriteList` assets.
    pub full_list: Option<ObjId>,
    /// See [`Abilities::full_list`].
    pub demo_list: Option<ObjId>,
}

/// One ability (`NamedSprite` entry of an ability icon list).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AbilityEntry {
    /// Index in its list.
    pub index: usize,
    /// `name`.
    pub name: String,
    /// Icon (`sprite`).
    pub sprite: Option<ObjId>,
    /// `associatedGameObject`: the ability prefab root, under `prefabs/`.
    pub prefab: Option<ObjId>,
    /// `isOffensiveAbility`.
    pub offensive: bool,
    /// `Ability` or `InstantAbility`: the base ability component of the
    /// prefab root.
    pub kind: Option<String>,
    /// The base ability component ID.
    pub component: Option<ObjId>,
    /// Its raw fields (`Cooldown`/`cooldown`, `OnlyCastableOnGround`, ...).
    pub fields: Fields,
    /// Script classes of the prefab root's components, in order.
    pub scripts: Vec<String>,
}

/// A ScriptableObject asset (`MonoBehaviour` without GameObject).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    /// Asset ID.
    pub id: ObjId,
    /// `m_Name`.
    pub name: String,
    /// Script class.
    pub script: String,
    /// Raw fields.
    pub fields: Fields,
}

/// A script component that lives in one scene (singleton UI handlers).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SceneComponent {
    /// Build index of the scene.
    pub scene: usize,
    /// GameObject ID.
    pub object: ObjId,
    /// Component ID.
    pub component: ObjId,
    /// Raw fields.
    pub fields: Fields,
}

/// `NamedSpriteList` entry.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NamedSprite {
    /// `name`.
    pub name: String,
    /// `sprite`.
    pub sprite: Option<ObjId>,
    /// `associatedGameObject`.
    pub object: Option<ObjId>,
    /// `isOffensiveAbility`.
    pub offensive: bool,
}

/// `globals.json`: the game's global ScriptableObjects and singleton
/// components.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Globals {
    /// `PlayerColors` (`playerColors[]`: per colour slot the materials and
    /// colours).
    pub player_colors: Option<Asset>,
    /// `TeamColors` (`teamColors[]`: `team`, `fill`, `border`, `saturated`).
    pub team_colors: Option<Asset>,
    /// `BeamColorList` (`beamColors[]`).
    pub beam_colors: Option<Asset>,
    /// `Settings` (`GameMode`).
    pub settings: Option<Asset>,
    /// Every `NamedSpriteList` by asset name (`AbilityIcons`,
    /// `AbilityIcons_demo`, `boulders`).
    pub sprite_lists: BTreeMap<String, Vec<NamedSprite>>,
    /// The asset IDs of [`Globals::sprite_lists`].
    pub sprite_list_ids: BTreeMap<String, ObjId>,
    /// Singleton script components by script class, first occurrence in
    /// build order: `CharacterStatsList`, `AbilityGrid`, `AudioManager`,
    /// `CharacterSelectHandler`, `MidGameAbilitySelect`, ...
    pub components: BTreeMap<String, SceneComponent>,
}

/// `localization.json`: the game's `LocalizationTable` asset. Text is
/// looked up by the lower-cased English text of a `TextMeshProUGUI` /
/// `TextMesh` with a `LocalizedText` component: its index in `en` selects
/// the string of the other languages.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Localization {
    /// Asset ID.
    pub id: Option<ObjId>,
    /// Language codes in field order (`en`, `jp`, `ko`, `fr`, ...).
    pub languages: Vec<String>,
    /// Strings per language, index-aligned with `en`.
    pub strings: BTreeMap<String, Vec<String>>,
    /// Font asset references (`jpFontAsset`, `enFontAsset_stroke`, ...) ->
    /// `fonts.json` id.
    pub fonts: BTreeMap<String, Option<ObjId>>,
}

impl Localization {
    /// Text of `en_text` (any case) in `lang`, if the table has it.
    pub fn text(&self, lang: &str, en_text: &str) -> Option<&str> {
        let key = en_text.to_lowercase();
        let i = self
            .strings
            .get("en")?
            .iter()
            .position(|s| s.to_lowercase() == key)?;
        self.strings.get(lang)?.get(i).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(
            prefab_file("sharedassets0.assets:12"),
            "prefabs/sharedassets0.assets_12.json"
        );
        assert_eq!(sanitize("a b/c:d"), "a_b_c_d");
    }

    #[test]
    fn hashes_and_curves() {
        assert_eq!(path_hash(""), 0);
        assert_eq!(path_hash("Base Layer"), 756_556_552);
        let c = AnimCurve {
            segments: vec![[0.0, -240.0, 180.0, 0.0, 0.0], [0.5, 0.0, 0.0, 0.0, 15.0]],
            ..Default::default()
        };
        assert_eq!(c.eval(-1.0), 0.0);
        assert!((c.eval(0.5) - 15.0).abs() < 1e-4);
    }

    #[test]
    fn shape_json_is_tagged() {
        let s = DetShape {
            component: "level6:253".into(),
            enabled: true,
            kind: DetShapeKind::RoundedRect {
                extents: [3 << 32, 1 << 30],
                radius: 5 << 31,
            },
            hierarchy_number: 135,
            min_scale: 1 << 31,
            max_scale: 5 << 32,
            manually_call_init: false,
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["kind"], "rounded_rect");
        assert_eq!(j["radius"], 5i64 << 31);
        assert_eq!(serde_json::from_value::<DetShape>(j).unwrap(), s);
    }

    #[test]
    fn localization_lookup() {
        let mut l = Localization::default();
        l.strings
            .insert("en".into(), vec!["Play".into(), "Quit".into()]);
        l.strings
            .insert("de".into(), vec!["Spielen".into(), "Beenden".into()]);
        assert_eq!(l.text("de", "QUIT"), Some("Beenden"));
        assert_eq!(l.text("fr", "quit"), None);
        assert_eq!(l.text("de", "nope"), None);
    }
}
