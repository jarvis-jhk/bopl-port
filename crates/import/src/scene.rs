//! Extraction helpers: object IDs, value-to-JSON conversion with resolved
//! references, and flattening of GameObject hierarchies (scenes and prefabs)
//! into [`format::Object`] lists with world transforms.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use serde_json::{Map as JMap, Value as J};
use unity_assets::classes::{GameObject, SpriteRenderer, Transform};
use unity_assets::{AssetDir, AssetFile, PPtr, Value};

use crate::format::{self, ObjId};

/// Classes whose raw fields are not copied into `components` (bulky or
/// already extracted into typed fields).
const NO_RAW_FIELDS: &[&str] = &["ParticleSystem", "ParticleSystemRenderer", "SpriteRenderer"];

/// Shared state of one import run.
pub struct Ctx<'a> {
    /// The game data.
    pub dir: &'a AssetDir,
    sorting: HashMap<u32, (String, usize)>,
    tags: Vec<String>,
    materials: Mutex<HashMap<ObjId, String>>,
    material_infos: Mutex<HashMap<ObjId, Option<crate::particles::MaterialInfo>>>,
}

/// Global ID of an object of `file`.
pub fn obj_id(file: &AssetFile, path_id: i64) -> ObjId {
    format!("{}:{}", file.name, path_id)
}

/// Global ID a PPtr points to (without opening the target file).
pub fn ref_id(file: &AssetFile, p: PPtr) -> Option<ObjId> {
    if p.is_null() {
        return None;
    }
    let name = if p.file_id == 0 {
        file.name.as_str()
    } else {
        file.meta.externals.get(p.file_id as usize - 1)?.file_name()
    };
    Some(format!("{name}:{}", p.path_id))
}

/// Shortest decimal that round-trips an `f32` (keeps JSON small and readable).
pub fn f32_json(f: f32) -> J {
    if !f.is_finite() {
        return J::Null;
    }
    let s = f.to_string();
    serde_json::Number::from_f64(s.parse().unwrap_or(f as f64)).map_or(J::Null, J::Number)
}

/// Converts a value tree to JSON; PPtrs become `{"$ref": id}` or `null`.
pub fn to_json(file: &AssetFile, v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::from(*i),
        Value::UInt(u) => J::from(*u),
        Value::Float(f) => f32_json(*f),
        Value::Double(d) => serde_json::Number::from_f64(*d).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::Array(b.iter().map(|&x| J::from(x)).collect()),
        Value::Array(a) => J::Array(a.iter().map(|x| to_json(file, x)).collect()),
        Value::Object(o) => {
            if o.type_name.starts_with("PPtr<") {
                return match v.as_pptr().and_then(|p| ref_id(file, p)) {
                    Some(id) => {
                        let mut m = JMap::new();
                        m.insert("$ref".into(), J::String(id));
                        J::Object(m)
                    }
                    None => J::Null,
                };
            }
            J::Object(
                o.fields
                    .iter()
                    .map(|(k, x)| (k.to_string(), to_json(file, x)))
                    .collect(),
            )
        }
    }
}

/// Raw fields of a MonoBehaviour without the object header.
pub fn script_fields(file: &AssetFile, v: &Value) -> J {
    let mut j = to_json(file, v);
    if let J::Object(m) = &mut j {
        for k in ["m_GameObject", "m_Enabled", "m_Script", "m_Name"] {
            m.remove(k);
        }
    }
    j
}

fn trs_matrix(t: &Transform) -> Mat4 {
    let [x, y, z, w] = t.local_rotation;
    let mut q = Quat::from_xyzw(x, y, z, w);
    q = if q.length_squared() > 0.0 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    Mat4::from_scale_rotation_translation(
        Vec3::from(t.local_scale),
        q,
        Vec3::from(t.local_position),
    )
}

/// World transform fields from a world matrix.
pub fn world_transform(m: &Mat4) -> format::WorldTransform {
    let (s, mut r, t) = m.to_scale_rotation_translation();
    let mut rotation_z = m.x_axis.y.atan2(m.x_axis.x);
    if !r.is_finite() {
        // Degenerate (zero) scale: keep the 2D rotation only.
        if m.x_axis.x == 0.0 && m.x_axis.y == 0.0 {
            rotation_z = (-m.y_axis.x).atan2(m.y_axis.y);
        }
        r = Quat::from_rotation_z(rotation_z);
    }
    format::WorldTransform {
        trs: format::Trs {
            position: finite(t.into()),
            rotation: r.to_array(),
            scale: finite(s.into()),
        },
        rotation_z: rotation_z.to_degrees(),
        matrix: finite([
            m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y, m.w_axis.x, m.w_axis.y,
        ]),
    }
}

/// Replaces non-finite values (which JSON cannot hold) by 0.
pub fn finite<const N: usize>(a: [f32; N]) -> [f32; N] {
    a.map(|v| if v.is_finite() { v } else { 0.0 })
}

impl<'a> Ctx<'a> {
    /// Builds the context (reads TagManager from `globalgamemanagers`).
    pub fn new(dir: &'a AssetDir) -> Result<Self> {
        let ggm = dir.file("globalgamemanagers")?;
        let mut sorting = HashMap::new();
        let mut tags = vec!["Untagged".to_string()];
        if let Some(o) = ggm.objects_of(78).next() {
            let tm = unity_assets::classes::TagManager::from_value(&dir.read(&ggm, o.path_id)?);
            for (i, l) in tm.sorting_layers.iter().enumerate() {
                sorting.insert(l.unique_id as u32, (l.name.clone(), i));
            }
            // Built-in tags 1..=6 precede custom tags (which start at 20000).
            tags.extend(
                [
                    "Respawn",
                    "Finish",
                    "EditorOnly",
                    "MainCamera",
                    "Player",
                    "GameController",
                ]
                .map(String::from),
            );
            tags.extend(tm.tags);
        }
        Ok(Ctx {
            dir,
            sorting,
            tags,
            materials: Default::default(),
            material_infos: Default::default(),
        })
    }

    /// Sorting layer (name, draw index) for a renderer's layer ID.
    pub fn sorting_layer(&self, id: i32) -> (String, usize) {
        self.sorting
            .get(&(id as u32))
            .cloned()
            .unwrap_or_else(|| ("Default".into(), 0))
    }

    /// Tag names by index (0 Untagged, 1..=6 built-in, then custom tags).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    fn tag_name(&self, tag: u16) -> String {
        let i = tag as usize;
        let custom = i.checked_sub(20000).map(|j| j + 7);
        self.tags
            .get(custom.unwrap_or(i))
            .cloned()
            .unwrap_or_else(|| format!("tag{tag}"))
    }

    fn material_name(&self, file: &AssetFile, p: PPtr) -> Option<String> {
        let id = ref_id(file, p)?;
        if let Some(n) = self.materials.lock().unwrap().get(&id) {
            return Some(n.clone());
        }
        let name = match self.dir.deref(file, p) {
            Ok(Some((_, _, v))) => v.get("m_Name").str().to_owned(),
            _ => String::new(),
        };
        self.materials.lock().unwrap().insert(id, name.clone());
        Some(name)
    }

    /// (name, shader, `_MainTex` texture id) of a material by id.
    fn material_info(&self, id: &str) -> Option<crate::particles::MaterialInfo> {
        if let Some(m) = self.material_infos.lock().unwrap().get(id) {
            return m.clone();
        }
        let info = (|| {
            let (file, pid) = id.rsplit_once(':')?;
            let f = self.dir.file(file).ok()?;
            let v = self.dir.read(&f, pid.parse().ok()?).ok()?;
            let shader = match v.get("m_Shader").as_pptr() {
                Some(p) if !p.is_null() => match self.dir.deref(&f, p) {
                    Ok(Some((_, _, sv))) => sv.path("m_ParsedForm.m_Name").str().to_owned(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let texture = v
                .path("m_SavedProperties.m_TexEnvs")
                .as_array()
                .iter()
                .find(|p| p.get("first").str() == "_MainTex")
                .and_then(|p| p.path("second.m_Texture").as_pptr())
                .and_then(|p| ref_id(&f, p));
            Some((v.get("m_Name").str().to_owned(), shader, texture))
        })();
        self.material_infos
            .lock()
            .unwrap()
            .insert(id.to_owned(), info.clone());
        info
    }

    /// Flattens the hierarchies under `roots` (GameObject path IDs of `file`)
    /// into a list where parents precede their children.
    pub fn flatten(&self, file: &AssetFile, roots: &[i64]) -> Result<Vec<format::Object>> {
        self.flatten_opts(file, roots, false)
    }

    /// Just the GameObject `go_id` (with its components, without children;
    /// its world transform is its local one).
    pub fn flatten_shallow(&self, file: &AssetFile, go_id: i64) -> Result<Vec<format::Object>> {
        let mut out = Vec::new();
        self.visit(
            file,
            go_id,
            None,
            Mat4::IDENTITY,
            true,
            false,
            false,
            &mut out,
        )?;
        Ok(out)
    }

    /// Like [`flatten`](Self::flatten); `raw_particles` keeps the raw fields
    /// of `ParticleSystem` and `ParticleSystemRenderer` components.
    pub fn flatten_opts(
        &self,
        file: &AssetFile,
        roots: &[i64],
        raw_particles: bool,
    ) -> Result<Vec<format::Object>> {
        let mut out = Vec::new();
        for &r in roots {
            self.visit(
                file,
                r,
                None,
                Mat4::IDENTITY,
                true,
                raw_particles,
                true,
                &mut out,
            )
            .with_context(|| format!("{}: hierarchy of {r}", file.name))?;
        }
        // Resolve sub-emitters to object indices.
        let emitters: HashMap<ObjId, usize> = out
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.particles.as_ref().map(|p| (p.component.clone(), i)))
            .collect();
        for o in &mut out {
            if let Some(p) = &mut o.particles {
                for s in &mut p.sub_emitters {
                    s.object = s.emitter.as_ref().and_then(|e| emitters.get(e).copied());
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        file: &AssetFile,
        go_id: i64,
        parent: Option<usize>,
        parent_world: Mat4,
        parent_active: bool,
        raw_particles: bool,
        recurse: bool,
        out: &mut Vec<format::Object>,
    ) -> Result<()> {
        let go = GameObject::from_value(&self.dir.read(file, go_id)?);
        let mut transform = Transform::default();
        let mut obj = format::Object {
            id: obj_id(file, go_id),
            name: go.name.clone(),
            parent,
            children: Vec::new(),
            active: go.active,
            active_in_hierarchy: go.active && parent_active,
            layer: go.layer,
            transform: None,
            tag: self.tag_name(go.tag),
            local: Default::default(),
            world: Default::default(),
            rect: None,
            sprite: None,
            fix: None,
            shape: None,
            particles: None,
            components: Vec::new(),
        };
        let mut ps_raw: Option<(ObjId, J)> = None;
        let mut psr_raw: Option<J> = None;
        for c in &go.components {
            // Components always live in the GameObject's file.
            let Some(info) = (c.file_id == 0).then(|| file.object(c.path_id)).flatten() else {
                continue;
            };
            let read = self.dir.read_object(file, info)?;
            let v = &read.value;
            let class = self
                .dir
                .registry()
                .class_name(info.class_id)
                .unwrap_or("Unknown")
                .to_owned();
            let cid = obj_id(file, info.path_id);
            match class.as_str() {
                "Transform" | "RectTransform" => {
                    transform = Transform::from_value(v);
                    obj.transform = Some(cid);
                    continue;
                }
                "SpriteRenderer" => obj.sprite = Some(self.sprite_renderer(file, v)),
                _ => {}
            }
            match read.script.as_ref().map(|k| k.class.as_str()) {
                Some("FixTransform") if obj.fix.is_none() => {
                    obj.fix = Some(fix_transform(cid.clone(), v))
                }
                Some(k @ ("DPhysicsBox" | "DPhysicsCircle" | "DPhysicsRoundedRect"))
                    if obj.shape.is_none() =>
                {
                    obj.shape = Some(det_shape(k, cid.clone(), v))
                }
                _ => {}
            }
            let particles = class == "ParticleSystem" || class == "ParticleSystemRenderer";
            if class == "ParticleSystem" && ps_raw.is_none() {
                ps_raw = Some((cid.clone(), to_json(file, v)));
            } else if class == "ParticleSystemRenderer" && psr_raw.is_none() {
                psr_raw = Some(to_json(file, v));
            }
            let fields = if NO_RAW_FIELDS.contains(&class.as_str()) && !(raw_particles && particles)
            {
                J::Null
            } else if read.script.is_some() {
                script_fields(file, v)
            } else {
                to_json(file, v)
            };
            let enabled = match v.get("m_Enabled") {
                Value::Null => None,
                e => Some(e.bool()),
            };
            obj.components.push(format::Component {
                id: cid,
                kind: class,
                script: read.script.as_ref().map(|k| k.full_name()),
                enabled,
                fields,
            });
        }
        if let Some((cid, ps)) = &ps_raw {
            obj.particles = Some(Box::new(crate::particles::from_raw(
                cid.clone(),
                ps,
                psr_raw.as_ref(),
                &|id| self.material_info(id),
                &|layer| self.sorting_layer(layer),
            )));
        }
        let world = parent_world * trs_matrix(&transform);
        obj.local = format::Trs {
            position: finite(transform.local_position),
            rotation: finite(transform.local_rotation),
            scale: finite(transform.local_scale),
        };
        obj.world = world_transform(&world);
        obj.rect = transform.rect.as_ref().map(|r| format::RectTransform {
            anchor_min: r.anchor_min,
            anchor_max: r.anchor_max,
            anchored_position: r.anchored_position,
            size_delta: r.size_delta,
            pivot: r.pivot,
        });
        let active = obj.active_in_hierarchy;
        let me = out.len();
        out.push(obj);
        if !recurse {
            return Ok(());
        }
        for ch in &transform.children {
            if ch.file_id != 0 {
                continue;
            }
            let t = Transform::from_value(&self.dir.read(file, ch.path_id)?);
            if t.game_object.is_null() {
                continue;
            }
            let idx = out.len();
            out[me].children.push(idx);
            self.visit(
                file,
                t.game_object.path_id,
                Some(me),
                world,
                active,
                raw_particles,
                true,
                out,
            )?;
        }
        Ok(())
    }

    fn sprite_renderer(&self, file: &AssetFile, v: &Value) -> format::SpriteRenderer {
        let s = SpriteRenderer::from_value(v);
        let (layer, index) = self.sorting_layer(s.common.sorting_layer_id);
        format::SpriteRenderer {
            enabled: s.common.enabled,
            sprite: ref_id(file, s.sprite),
            color: s.color,
            flip_x: s.flip_x,
            flip_y: s.flip_y,
            draw_mode: s.draw_mode,
            size: s.size,
            sorting_layer: layer,
            sorting_layer_index: index,
            sorting_order: s.common.sorting_order as i32,
            mask_interaction: s.mask_interaction,
            materials: s
                .common
                .materials
                .iter()
                .filter_map(|&m| self.material_name(file, m))
                .collect(),
            material_ids: s
                .common
                .materials
                .iter()
                .filter_map(|&m| ref_id(file, m))
                .collect(),
        }
    }
}

/// Raw bits of a `Fix` (`{m_rawValue}`).
pub fn fix(v: &Value) -> i64 {
    v.get("m_rawValue").i64()
}

/// Raw bits of a `Vec2` of `Fix`.
pub fn fix2(v: &Value) -> [i64; 2] {
    [fix(v.get("x")), fix(v.get("y"))]
}

fn fix_transform(component: ObjId, v: &Value) -> format::FixTransform {
    format::FixTransform {
        component,
        position: fix2(v.get("_position")),
        rotation: fix(v.get("_rotation")),
        up: fix2(v.get("_up")),
        right: fix2(v.get("_right")),
        offset: fix2(v.get("offset")),
    }
}

fn det_shape(class: &str, component: ObjId, v: &Value) -> format::DetShape {
    let kind = match class {
        "DPhysicsBox" => format::DetShapeKind::Box {
            extents: fix2(v.get("startExtents")),
            minimum_extents: fix(v.get("minimumExtents")),
        },
        "DPhysicsCircle" => format::DetShapeKind::Circle {
            radius: fix(v.get("startRadius")),
        },
        _ => format::DetShapeKind::RoundedRect {
            extents: fix2(v.get("startExtents")),
            radius: fix(v.get("startRadius")),
        },
    };
    format::DetShape {
        component,
        enabled: v.get("m_Enabled").bool(),
        kind,
        hierarchy_number: v.get("hierarchyNumber").i64(),
        min_scale: fix(v.get("MinScale")),
        max_scale: fix(v.get("MaxScale")),
        manually_call_init: v.get("manuallyCallInit").bool(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_scale_is_finite() {
        let m = Mat4::from_scale_rotation_translation(
            Vec3::new(0.0, 2.0, 1.0),
            Quat::IDENTITY,
            Vec3::X,
        );
        let w = world_transform(&m);
        assert!(w.trs.rotation.iter().all(|v| v.is_finite()));
        assert_eq!(w.trs.position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn floats_are_short() {
        assert_eq!(f32_json(0.1).to_string(), "0.1");
        assert_eq!(f32_json(f32::INFINITY), J::Null);
    }

    #[test]
    fn world_matrix_composition() {
        let parent = Transform {
            local_position: [1.0, 2.0, 0.0],
            local_rotation: [
                0.0,
                0.0,
                (45f32.to_radians() / 2.0).sin(),
                (45f32.to_radians() / 2.0).cos(),
            ],
            local_scale: [2.0, 2.0, 1.0],
            ..Default::default()
        };
        let child = Transform {
            local_position: [1.0, 0.0, 0.0],
            local_rotation: [0.0, 0.0, 0.0, 1.0],
            local_scale: [1.0, 1.0, 1.0],
            ..Default::default()
        };
        let w = world_transform(&(trs_matrix(&parent) * trs_matrix(&child)));
        let d = 2f32.sqrt();
        assert!((w.trs.position[0] - (1.0 + d)).abs() < 1e-5);
        assert!((w.trs.position[1] - (2.0 + d)).abs() < 1e-5);
        assert!((w.rotation_z - 45.0).abs() < 1e-4);
        assert!((w.trs.scale[0] - 2.0).abs() < 1e-5);
        assert!((w.matrix[0] - d).abs() < 1e-5 && (w.matrix[1] - d).abs() < 1e-5);
    }
}
