//! Typed views of the engine classes the importer relies on, built from the
//! generic [`Value`] trees. Field meanings follow Unity's serialization names.

use crate::value::{PPtr, Value};

fn pptr(v: &Value) -> PPtr {
    v.as_pptr().unwrap_or_default()
}

/// `GameObject` (class 1).
#[derive(Clone, Debug, Default)]
pub struct GameObject {
    /// Object name.
    pub name: String,
    /// Physics/render layer index.
    pub layer: u32,
    /// Tag index into `TagManager` (0 = Untagged).
    pub tag: u16,
    /// `activeSelf`.
    pub active: bool,
    /// Components, the Transform first.
    pub components: Vec<PPtr>,
}

impl GameObject {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        GameObject {
            name: v.get("m_Name").str().to_owned(),
            layer: v.get("m_Layer").i64() as u32,
            tag: v.get("m_Tag").i64() as u16,
            active: v.get("m_IsActive").bool(),
            components: v
                .get("m_Component")
                .as_array()
                .iter()
                .map(|c| pptr(c.get("component")))
                .collect(),
        }
    }
}

/// `Transform` (class 4) and the shared part of `RectTransform` (224).
#[derive(Clone, Debug, Default)]
pub struct Transform {
    /// Owner.
    pub game_object: PPtr,
    /// Local rotation quaternion `[x, y, z, w]`.
    pub local_rotation: [f32; 4],
    /// Local position.
    pub local_position: [f32; 3],
    /// Local scale.
    pub local_scale: [f32; 3],
    /// Child transforms in order.
    pub children: Vec<PPtr>,
    /// Parent transform (null at the root).
    pub father: PPtr,
    /// RectTransform-only layout data.
    pub rect: Option<RectLayout>,
}

/// `RectTransform` layout fields.
#[derive(Clone, Debug, Default)]
pub struct RectLayout {
    /// Anchor minimum (normalised in parent rect).
    pub anchor_min: [f32; 2],
    /// Anchor maximum.
    pub anchor_max: [f32; 2],
    /// Pivot position relative to the anchors.
    pub anchored_position: [f32; 2],
    /// Size relative to the anchors.
    pub size_delta: [f32; 2],
    /// Pivot (normalised in own rect).
    pub pivot: [f32; 2],
}

impl Transform {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        let rect = (!v.get("m_AnchorMin").is_null()).then(|| RectLayout {
            anchor_min: v.get("m_AnchorMin").vec2(),
            anchor_max: v.get("m_AnchorMax").vec2(),
            anchored_position: v.get("m_AnchoredPosition").vec2(),
            size_delta: v.get("m_SizeDelta").vec2(),
            pivot: v.get("m_Pivot").vec2(),
        });
        Transform {
            game_object: pptr(v.get("m_GameObject")),
            local_rotation: v.get("m_LocalRotation").vec4(),
            local_position: v.get("m_LocalPosition").vec3(),
            local_scale: v.get("m_LocalScale").vec3(),
            children: v.get("m_Children").as_array().iter().map(pptr).collect(),
            father: pptr(v.get("m_Father")),
            rect,
        }
    }
}

/// Renderer fields shared by `SpriteRenderer`, `LineRenderer`, `SpriteMask`, ...
#[derive(Clone, Debug, Default)]
pub struct RendererCommon {
    /// Owner.
    pub game_object: PPtr,
    /// Enabled flag.
    pub enabled: bool,
    /// Materials.
    pub materials: Vec<PPtr>,
    /// Sorting layer unique ID (see `TagManager`).
    pub sorting_layer_id: i32,
    /// Order within the sorting layer.
    pub sorting_order: i16,
}

impl RendererCommon {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        RendererCommon {
            game_object: pptr(v.get("m_GameObject")),
            enabled: v.get("m_Enabled").bool(),
            materials: v.get("m_Materials").as_array().iter().map(pptr).collect(),
            sorting_layer_id: v.get("m_SortingLayerID").i64() as i32,
            sorting_order: v.get("m_SortingOrder").i64() as i16,
        }
    }
}

/// `SpriteRenderer` (class 212).
#[derive(Clone, Debug, Default)]
pub struct SpriteRenderer {
    /// Shared renderer fields.
    pub common: RendererCommon,
    /// Sprite drawn.
    pub sprite: PPtr,
    /// Tint `[r, g, b, a]`.
    pub color: [f32; 4],
    /// Mirror horizontally.
    pub flip_x: bool,
    /// Mirror vertically.
    pub flip_y: bool,
    /// 0 = simple, 1 = sliced, 2 = tiled.
    pub draw_mode: i32,
    /// Size in world units for sliced/tiled modes.
    pub size: [f32; 2],
    /// 0 = none, 1 = visible inside mask, 2 = visible outside mask.
    pub mask_interaction: i32,
}

impl SpriteRenderer {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        SpriteRenderer {
            common: RendererCommon::from_value(v),
            sprite: pptr(v.get("m_Sprite")),
            color: v.get("m_Color").color(),
            flip_x: v.get("m_FlipX").bool(),
            flip_y: v.get("m_FlipY").bool(),
            draw_mode: v.get("m_DrawMode").i64() as i32,
            size: v.get("m_Size").vec2(),
            mask_interaction: v.get("m_MaskInteraction").i64() as i32,
        }
    }
}

/// `Sprite` (class 213).
#[derive(Clone, Debug, Default)]
pub struct Sprite {
    /// Asset name.
    pub name: String,
    /// Rectangle in the source texture, pixels, origin bottom-left.
    pub rect: [f32; 4],
    /// `m_Offset` as stored by Unity (pivot offset from the rect centre, pixels).
    pub offset: [f32; 2],
    /// 9-slice border `[left, bottom, right, top]` in pixels.
    pub border: [f32; 4],
    /// Pixels per world unit.
    pub pixels_to_units: f32,
    /// Pivot, normalised in `rect`.
    pub pivot: [f32; 2],
    /// Texture the sprite samples (an atlas page if packed).
    pub texture: PPtr,
    /// Separate alpha texture, if any.
    pub alpha_texture: PPtr,
    /// Rectangle inside `texture`, pixels, origin bottom-left.
    pub texture_rect: [f32; 4],
    /// Offset of `texture_rect` inside `rect` (tight packing trims).
    pub texture_rect_offset: [f32; 2],
    /// Packed flag (bit 0), packing mode (bit 1: tight), rotation (bits 2..5).
    pub settings_raw: u32,
    /// Sprite atlas, if packed by one.
    pub atlas: PPtr,
    /// Atlas tags.
    pub atlas_tags: Vec<String>,
    /// Physics shape outlines (pixels relative to pivot / ppu = units).
    pub physics_shape: Vec<Vec<[f32; 2]>>,
}

impl Sprite {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        let rd = v.get("m_RD");
        Sprite {
            name: v.get("m_Name").str().to_owned(),
            rect: v.get("m_Rect").rect(),
            offset: v.get("m_Offset").vec2(),
            border: v.get("m_Border").vec4(),
            pixels_to_units: v.get("m_PixelsToUnits").f32(),
            pivot: v.get("m_Pivot").vec2(),
            texture: pptr(rd.get("texture")),
            alpha_texture: pptr(rd.get("alphaTexture")),
            texture_rect: rd.get("textureRect").rect(),
            texture_rect_offset: rd.get("textureRectOffset").vec2(),
            settings_raw: rd.get("settingsRaw").i64() as u32,
            atlas: pptr(v.get("m_SpriteAtlas")),
            atlas_tags: v
                .get("m_AtlasTags")
                .as_array()
                .iter()
                .map(|t| t.str().to_owned())
                .collect(),
            physics_shape: v
                .get("m_PhysicsShape")
                .as_array()
                .iter()
                .map(|p| p.as_array().iter().map(|q| q.vec2()).collect())
                .collect(),
        }
    }

    /// Whether the sprite is packed into an atlas.
    pub fn packed(&self) -> bool {
        self.settings_raw & 1 != 0
    }

    /// Packing rotation (0 none, 1 flip H, 2 flip V, 3 rotate 180, 4 rotate 90).
    pub fn packing_rotation(&self) -> u32 {
        (self.settings_raw >> 2) & 0xf
    }

    /// Whether tight (mesh) packing was used.
    pub fn tight(&self) -> bool {
        self.settings_raw & 2 != 0
    }
}

/// Location of data in a resource file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamRef {
    /// Path as stored (`*.resS` / `*.resource`), empty if inline.
    pub path: String,
    /// Byte offset.
    pub offset: u64,
    /// Byte size.
    pub size: u64,
}

/// `Texture2D` (class 28).
#[derive(Clone, Debug, Default)]
pub struct Texture2D {
    /// Asset name.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `TextureFormat` enum value.
    pub format: i32,
    /// Number of mip levels.
    pub mip_count: u32,
    /// Filter mode: 0 point, 1 bilinear, 2 trilinear.
    pub filter_mode: i32,
    /// Wrap mode U: 0 repeat, 1 clamp, ...
    pub wrap_u: i32,
    /// Inline image data (empty when streamed).
    pub image_data: Vec<u8>,
    /// Streamed image data.
    pub stream: StreamRef,
}

impl Texture2D {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        let s = v.get("m_StreamData");
        let ts = v.get("m_TextureSettings");
        Texture2D {
            name: v.get("m_Name").str().to_owned(),
            width: v.get("m_Width").i64() as u32,
            height: v.get("m_Height").i64() as u32,
            format: v.get("m_TextureFormat").i64() as i32,
            mip_count: v.get("m_MipCount").i64() as u32,
            filter_mode: ts.get("m_FilterMode").i64() as i32,
            wrap_u: ts.get("m_WrapU").i64() as i32,
            image_data: v.get("image data").as_bytes().unwrap_or_default().to_vec(),
            stream: StreamRef {
                path: s.get("path").str().to_owned(),
                offset: s.get("offset").as_u64().unwrap_or(0),
                size: s.get("size").as_u64().unwrap_or(0),
            },
        }
    }
}

/// `AudioClip` (class 83).
#[derive(Clone, Debug, Default)]
pub struct AudioClip {
    /// Asset name.
    pub name: String,
    /// 0 decompress on load, 1 compressed in memory, 2 streaming.
    pub load_type: i32,
    /// Channel count.
    pub channels: u32,
    /// Sample rate.
    pub frequency: u32,
    /// Length in seconds.
    pub length: f32,
    /// 0 PCM, 1 Vorbis, 2 ADPCM, 3 MP3, ...
    pub compression_format: i32,
    /// FSB5 bank location.
    pub resource: StreamRef,
}

impl AudioClip {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        let r = v.get("m_Resource");
        AudioClip {
            name: v.get("m_Name").str().to_owned(),
            load_type: v.get("m_LoadType").i64() as i32,
            channels: v.get("m_Channels").i64() as u32,
            frequency: v.get("m_Frequency").i64() as u32,
            length: v.get("m_Length").f32(),
            compression_format: v.get("m_CompressionFormat").i64() as i32,
            resource: StreamRef {
                path: r.get("m_Source").str().to_owned(),
                offset: r.get("m_Offset").as_u64().unwrap_or(0),
                size: r.get("m_Size").as_u64().unwrap_or(0),
            },
        }
    }
}

/// A sorting layer from `TagManager`.
#[derive(Clone, Debug, Default)]
pub struct SortingLayer {
    /// Layer name.
    pub name: String,
    /// The ID renderers store in `m_SortingLayerID`.
    pub unique_id: i64,
}

/// `TagManager` (class 78).
#[derive(Clone, Debug, Default)]
pub struct TagManager {
    /// Custom tags; `GameObject::tag` 20000 + i refers to `tags[i]`.
    pub tags: Vec<String>,
    /// Layer names by index (32).
    pub layers: Vec<String>,
    /// Sorting layers in draw order.
    pub sorting_layers: Vec<SortingLayer>,
}

impl TagManager {
    /// Reads from a value tree.
    pub fn from_value(v: &Value) -> Self {
        let strs = |k: &str| {
            v.get(k)
                .as_array()
                .iter()
                .map(|s| s.str().to_owned())
                .collect()
        };
        TagManager {
            tags: strs("tags"),
            layers: strs("layers"),
            sorting_layers: v
                .get("m_SortingLayers")
                .as_array()
                .iter()
                .map(|l| SortingLayer {
                    name: l.get("name").str().to_owned(),
                    unique_id: l.get("uniqueID").i64(),
                })
                .collect(),
        }
    }
}

/// Scene paths from `BuildSettings` (class 141), in build-index order.
pub fn build_scenes(v: &Value) -> Vec<String> {
    v.get("scenes")
        .as_array()
        .iter()
        .map(|s| s.str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Object;

    fn obj(f: Vec<(&str, Value)>) -> Value {
        Value::Object(Object {
            type_name: "T".into(),
            fields: f.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        })
    }

    #[test]
    fn sprite_settings_bits() {
        let s = Sprite {
            settings_raw: 1 | 2 | (4 << 2),
            ..Default::default()
        };
        assert!(s.packed() && s.tight());
        assert_eq!(s.packing_rotation(), 4);
    }

    #[test]
    fn game_object_components() {
        let p = |id| {
            Value::Object(Object {
                type_name: "PPtr<Component>".into(),
                fields: vec![
                    ("m_FileID".into(), Value::Int(0)),
                    ("m_PathID".into(), Value::Int(id)),
                ],
            })
        };
        let v = obj(vec![
            (
                "m_Component",
                Value::Array(vec![
                    obj(vec![("component", p(5))]),
                    obj(vec![("component", p(6))]),
                ]),
            ),
            ("m_Layer", Value::UInt(9)),
            ("m_Name", Value::String("Box".into())),
            ("m_IsActive", Value::Bool(true)),
        ]);
        let g = GameObject::from_value(&v);
        assert_eq!(g.name, "Box");
        assert_eq!(g.layer, 9);
        assert!(g.active);
        assert_eq!(
            g.components.iter().map(|c| c.path_id).collect::<Vec<_>>(),
            [5, 6]
        );
    }
}
