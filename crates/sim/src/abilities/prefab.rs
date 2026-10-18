//! Prefab and content lookups shared by the ability modules: finding
//! prefabs, components and animations by import reference, reading
//! serialized fields (raw Fix, float-path Fix, vectors, curves) and the
//! hierarchy-number bookkeeping of prefabs. No simulation logic.

use std::sync::Arc;

use crate::curve::Curve;
use crate::data::{AbilityDef, AnimRef, Content, Prefab, PrefabComponent, PrefabObject, Value};
use crate::fix::Fix;
use crate::object::ObjId;
use crate::vec2::Vec2;
use crate::view::Visual;
use crate::world::World;

/// A shared `Value::Null` for absent fields.
pub static NULL: Value = Value::Null;

// -- prefabs by reference -------------------------------------------------------

/// Index of the prefab a reference (`"<file>:<path id>"` of a prefab object
/// or of one of its components) belongs to.
pub fn prefab_index(w: &World, r: &str) -> Option<usize> {
    prefab_by_ref(&w.content, Some(r))
}

/// [`prefab_index`] on the content (`None` reference → `None`).
pub fn prefab_by_ref(c: &Content, r: Option<&str>) -> Option<usize> {
    let r = r?;
    if let Some(&i) = c.prefab_index.get(r) {
        return Some(i);
    }
    c.prefabs.iter().position(|p| {
        p.objects
            .iter()
            .any(|o| o.components.iter().any(|k| k.id == r))
    })
}

/// The prefab referenced by field `key` of `fields`.
pub fn prefab_field(w: &World, fields: &Value, key: &str) -> Option<usize> {
    prefab_index(w, fields.reference(key)?)
}

/// Index of a prefab by reference, else by name.
pub fn prefab_ref(c: &Content, r: Option<&str>, name: &str) -> Option<usize> {
    prefab_by_ref(c, r).or_else(|| c.prefabs.iter().position(|p| p.name == name))
}

/// Number of updatables (hierarchy numbers) of a prefab.
pub fn updatables_of(w: &World, prefab: usize) -> i32 {
    w.content
        .prefabs
        .get(prefab)
        .map_or(0, |p| p.updatable_count())
}

/// Updatables of a prefab by reference (0 if unresolved).
pub fn prefab_updatables(w: &World, r: &str) -> i32 {
    prefab_index(w, r).map_or(0, |p| updatables_of(w, p))
}

/// Updatables of an optional prefab.
pub fn updatables(c: &Content, p: Option<usize>) -> i32 {
    p.and_then(|i| c.prefabs.get(i))
        .map_or(0, |p| p.updatable_count())
}

/// The fixed-transform position stored in a prefab's root (effect offsets).
pub fn prefab_position(w: &World, prefab: usize) -> Vec2 {
    root_position(&w.content, Some(prefab))
}

/// [`prefab_position`] of an optional prefab.
pub fn root_position(c: &Content, p: Option<usize>) -> Vec2 {
    p.and_then(|i| c.prefabs.get(i))
        .and_then(|p| p.objects.first())
        .and_then(|o| o.fix)
        .map_or(Vec2::ZERO, |t| t.position)
}

/// Updatables and root position of a referenced prefab.
pub fn effect_info(c: &Content, r: Option<&str>) -> (i32, Vec2) {
    let p = prefab_by_ref(c, r);
    (updatables(c, p), root_position(c, p))
}

// -- components ------------------------------------------------------------------

/// The ability's prefab.
pub fn ability_prefab<'a>(w: &'a World, def: &AbilityDef) -> Option<&'a Prefab> {
    def.prefab.and_then(|i| w.content.prefabs.get(i))
}

/// The prefab root object of an ability.
pub fn ability_root<'a>(w: &'a World, def: &AbilityDef) -> Option<&'a PrefabObject> {
    ability_prefab(w, def).and_then(|p| p.objects.first())
}

/// A component on the ability prefab's root object.
pub fn root_component<'a>(
    c: &'a Content,
    def: &AbilityDef,
    class: &str,
) -> Option<&'a PrefabComponent> {
    c.prefabs
        .get(def.prefab?)?
        .objects
        .first()?
        .component(class)
}

/// Fields of a component on the ability prefab root (`Value::Null` if absent).
pub fn component_fields(w: &World, def: &AbilityDef, class: &str) -> Value {
    root_component(&w.content, def, class).map_or(Value::Null, |c| c.fields.clone())
}

/// The first component of a class anywhere in a prefab.
pub fn component<'a>(c: &'a Content, prefab: usize, class: &str) -> Option<&'a PrefabComponent> {
    c.prefabs.get(prefab)?.find_component(class).map(|(_, c)| c)
}

/// Fields of the first component of a class on the prefab root.
pub fn root_fields<'a>(p: &'a Prefab, class: &str) -> Option<&'a Value> {
    p.objects.first()?.component(class).map(|c| &c.fields)
}

/// Fields of the first component of a class anywhere in the prefab.
pub fn any_fields<'a>(p: &'a Prefab, class: &str) -> Option<&'a Value> {
    p.find_component(class).map(|(_, c)| &c.fields)
}

/// Fields of the first component of a class (`Null` if absent).
pub fn fields<'a>(p: &'a Prefab, class: &str) -> &'a Value {
    p.find_component(class).map_or(&NULL, |(_, c)| &c.fields)
}

/// Hierarchy number of the first component of a class.
pub fn hier_of(p: &Prefab, class: &str) -> Option<i32> {
    p.find_component(class).and_then(|(_, c)| c.hierarchy)
}

/// The `hierarchyNumber` field of a component's fields.
pub fn hier_field(fields: &Value) -> i32 {
    fields.int("hierarchyNumber").unwrap_or(0) as i32
}

/// The root `SpriteAnimator`'s `AlwaysStopDuringTimeStop` of a prefab.
pub fn root_always_stop(p: &Prefab) -> bool {
    root_fields(p, "SpriteAnimator")
        .and_then(|a| a.boolean("AlwaysStopDuringTimeStop"))
        .unwrap_or(false)
}

/// The prefab object an object was built from (`Visual::Prefab`).
pub fn prefab_object(w: &World, obj: ObjId) -> Option<&PrefabObject> {
    match w.objects.get(obj)?.visual {
        Visual::Prefab { prefab, object } => w
            .content
            .prefabs
            .get(prefab as usize)?
            .objects
            .get(object as usize),
        _ => None,
    }
}

/// The Unity tag of an object: its prefab object's, or the level
/// platform's / static collider's (`""` if unknown).
pub fn object_tag(w: &World, id: ObjId) -> &str {
    let Some(o) = w.objects.get(id) else {
        return "";
    };
    match o.visual {
        Visual::Prefab { .. } => prefab_object(w, id).map_or("", |o| o.tag.as_str()),
        Visual::Platform(i) => w
            .level
            .as_ref()
            .and_then(|l| l.platforms.get(i as usize))
            .map_or("", |p| p.tag.as_str()),
        Visual::Static(i) => w
            .level
            .as_ref()
            .and_then(|l| l.statics.get(i as usize))
            .map_or("", |s| s.tag.as_str()),
        _ => "",
    }
}

// -- field values ------------------------------------------------------------------

/// A `Fix` field (raw), or `d`.
pub fn fx(fields: &Value, key: &str, d: Fix) -> Fix {
    fields.fix(key).unwrap_or(d)
}

/// `(Fix)f` of a float-typed serialized value (float path, abilities.md §1).
pub fn float_fix(v: &Value) -> Option<Fix> {
    match v {
        Value::Float(b) => Some(Fix::from_f32(f32::from_bits(*b))),
        Value::Int(i) => Some(Fix::from_f32(*i as f32)),
        Value::Fix(f) => Some(*f),
        _ => None,
    }
}

/// A float field as Fix through the float path, or `d`.
pub fn float_fx(fields: &Value, key: &str, d: Fix) -> Fix {
    fields.get(key).and_then(float_fix).unwrap_or(d)
}

/// The last key time of a Unity float curve field, as `(Fix)time`.
pub fn float_curve_last_time(v: &Value) -> Option<Fix> {
    v.list("m_Curve")
        .last()
        .and_then(|k| k.get("time"))
        .and_then(float_fix)
}

fn raw_vec2(v: &Value) -> Option<Vec2> {
    match v {
        Value::Vec2(v) => Some(*v),
        Value::List(l) if l.len() == 2 => match (&l[0], &l[1]) {
            (Value::Int(a), Value::Int(b)) => Some(Vec2::raw(*a, *b)),
            _ => None,
        },
        _ => None,
    }
}

/// A `Vec2` field given as `{x, y}` or `[x, y]` raws.
pub fn vec2_field(fields: &Value, key: &str) -> Option<Vec2> {
    raw_vec2(fields.get(key)?)
}

/// A list of `Vec2` (raw) field.
pub fn vec2_list(fields: &Value, key: &str) -> Vec<Vec2> {
    fields.list(key).iter().filter_map(raw_vec2).collect()
}

/// A `Fix` field of an optional component, with a fallback.
pub fn fix_field(c: Option<&PrefabComponent>, key: &str, d: Fix) -> Fix {
    c.map_or(d, |c| c.fields.fix_or(key, d))
}

/// An int field of an optional component.
pub fn int_field(c: Option<&PrefabComponent>, key: &str, d: i64) -> i64 {
    c.and_then(|c| c.fields.int(key)).unwrap_or(d)
}

/// A `Vec2` field of an optional component.
pub fn vec2_field_or(c: Option<&PrefabComponent>, key: &str, d: Vec2) -> Vec2 {
    c.and_then(|c| c.fields.vec2(key)).unwrap_or(d)
}

/// A curve field of an optional component (empty if missing).
pub fn curve_field(c: Option<&PrefabComponent>, key: &str) -> Arc<Curve> {
    c.and_then(|c| c.fields.curve(key)).unwrap_or_default()
}

/// The raw field value of an optional component.
pub fn field<'a>(c: Option<&'a PrefabComponent>, key: &str) -> Option<&'a Value> {
    c.and_then(|c| c.fields.get(key))
}

// -- animations ---------------------------------------------------------------------

/// Index of an `AnimationData` asset by import id.
pub fn anim_data_index(w: &World, id: &str) -> Option<u16> {
    anim_data(&w.content, Some(id))
}

/// The `AnimationData` asset referenced by id.
pub fn anim_data(c: &Content, r: Option<&str>) -> Option<u16> {
    let r = r?;
    c.anim_data.iter().position(|a| a.id == r).map(|i| i as u16)
}

/// The `AnimationData` index of a reference field.
pub fn anim_data_field(w: &World, fields: &Value, key: &str) -> Option<u16> {
    anim_data(&w.content, fields.reference(key))
}

/// An animation by name (a missing one maps to animation 0 of the data, so
/// a broken import cannot panic the sim).
pub fn anim(w: &World, data: u16, name: &str) -> AnimRef {
    w.content
        .anim_ref(data, name)
        .unwrap_or(AnimRef { data, anim: 0 })
}

/// An animation by name, `None` if missing.
pub fn anim_named(c: &Content, data: Option<u16>, name: &str) -> Option<AnimRef> {
    c.anim_ref(data?, name)
}

/// Sound names are presentation; cues need `'static` names.
pub fn static_sound(s: &str) -> Option<&'static str> {
    const KNOWN: &[&str] = &[
        "prepareToQuantum",
        "reload2",
        "laserShoot",
        "fireRaygun",
        "magnetEnter",
    ];
    KNOWN.iter().find(|k| **k == s).copied()
}

/// A float `{x, y}` field (presentation data) through the float path.
pub fn float_vec2(v: &Value) -> Option<Vec2> {
    Some(Vec2::new(float_fix(v.get("x")?)?, float_fix(v.get("y")?)?))
}

/// The fields of a component class on an object's prefab object.
pub fn object_fields<'a>(w: &'a World, obj: ObjId, class: &str) -> Option<&'a Value> {
    prefab_object(w, obj)?.component(class).map(|c| &c.fields)
}

/// A raw `Vec2` value (`{x: Fix, y: Fix}`).
pub fn vec2_field_value(v: &Value) -> Option<Vec2> {
    match v {
        Value::Vec2(v) => Some(*v),
        _ => None,
    }
}
