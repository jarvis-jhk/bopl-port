//! `bopl-content`: turns bopl-port's imported game data (`docs/formats/import.md`)
//! into `bopl-sim` data: sprites, frame animations, generic prefabs, the
//! typed player and pickup prefabs, the ability list and levels. Fixed-point
//! values are copied as raw Q32.32 bits; Unity floats never reach the sim.

pub mod replay;
pub mod scenario;
pub mod spawns;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bopl_import::GameData;
use bopl_import::format::{self, DetShapeKind, Object};
use bopl_sim::abilities::pickup::PickupDef;
use bopl_sim::curve::{Curve, Key, Wrap};
use bopl_sim::data::*;
use bopl_sim::fix::Fix;
use bopl_sim::physics::shapes::ShapeKind;
use bopl_sim::physics::{BodyDef, ColliderDef, PhysParams};
use bopl_sim::vec2::Vec2;
use serde_json::Value as J;

/// Imported data plus the sim content built from it.
pub struct Loaded {
    pub game: GameData,
    pub content: Arc<Content>,
}

/// Default import dir for development (`cache/import`), else the user data dir.
pub fn default_import_dir() -> std::path::PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dev = root.join("cache/import");
    if dev.join("index.json").exists() {
        return dev;
    }
    bopl_import::paths::import_dir().unwrap_or(dev)
}

/// Loads an import dir and builds the content.
pub fn load(dir: impl AsRef<Path>) -> Result<Loaded> {
    let game = GameData::load(dir)?;
    let content = Arc::new(build_content(&game)?);
    Ok(Loaded { game, content })
}

// -- generic values ---------------------------------------------------------------

fn raw(j: &J) -> Option<i64> {
    j.get("m_rawValue")?.as_i64()
}

fn fixv(j: &J) -> Option<Fix> {
    raw(j).map(Fix)
}

fn vec2(j: &J) -> Option<Vec2> {
    Some(Vec2::new(fixv(j.get("x")?)?, fixv(j.get("y")?)?))
}

fn vraw(a: &[i64; 2]) -> Vec2 {
    Vec2::raw(a[0], a[1])
}

/// An `AnimationCurveFixed` field.
pub fn curve(j: &J) -> Option<Curve> {
    let keys = j.get("keys")?.as_array()?;
    let anim = j.get("Anim");
    let pre = anim
        .and_then(|a| a.get("m_PreInfinity"))
        .and_then(J::as_i64)
        .unwrap_or(2);
    let post = anim
        .and_then(|a| a.get("m_PostInfinity"))
        .and_then(J::as_i64)
        .unwrap_or(2);
    Some(Curve {
        keys: keys
            .iter()
            .map(|k| Key {
                time: fixv(&k["time"]).unwrap_or_default(),
                value: fixv(&k["value"]).unwrap_or_default(),
                in_tangent: fixv(&k["inTangent"]).unwrap_or_default(),
                out_tangent: fixv(&k["outTangent"]).unwrap_or_default(),
            })
            .collect(),
        pre: Wrap::from_code(pre),
        post: Wrap::from_code(post),
    })
}

/// Converts raw component fields into the sim's generic [`Value`].
pub fn value(j: &J) -> Value {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float((n.as_f64().unwrap_or(0.0) as f32).to_bits()),
        },
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::List(a.iter().map(value).collect()),
        J::Object(m) => {
            if let Some(r) = raw(j)
                && m.len() == 1
            {
                return Value::Fix(Fix(r));
            }
            if let Some(J::String(r)) = m.get("$ref") {
                return Value::Ref(r.clone());
            }
            if m.len() == 2
                && let Some(v) = vec2(j)
            {
                return Value::Vec2(v);
            }
            if m.contains_key("keys")
                && m.contains_key("Anim")
                && let Some(c) = curve(j)
            {
                return Value::Curve(Arc::new(c));
            }
            if m.len() == 1
                && let Some(b) = m.get("m_Bits").and_then(J::as_i64)
            {
                return Value::Int(b);
            }
            Value::Map(m.iter().map(|(k, v)| (k.clone(), value(v))).collect())
        }
    }
}

fn fx(f: &J, k: &str) -> Fix {
    f.get(k).and_then(fixv).unwrap_or_default()
}

fn fx_or(f: &J, k: &str, d: Fix) -> Fix {
    f.get(k).and_then(fixv).unwrap_or(d)
}

fn int(f: &J, k: &str) -> i64 {
    f.get(k).and_then(J::as_i64).unwrap_or(0)
}

fn flag(f: &J, k: &str) -> bool {
    int(f, k) != 0
}

fn hier(f: &J) -> i32 {
    int(f, "hierarchyNumber") as i32
}

// -- objects -------------------------------------------------------------------------

fn transform(o: &Object) -> TransformDef {
    match &o.fix {
        Some(t) => TransformDef {
            position: vraw(&t.position),
            rotation: Fix(t.rotation),
            right: vraw(&t.right),
            up: vraw(&t.up),
            offset: vraw(&t.offset),
        },
        None => TransformDef {
            right: Vec2::RIGHT,
            up: Vec2::UP,
            ..TransformDef::default()
        },
    }
}

fn body_def(o: &Object) -> Option<BodyDef> {
    let c = o.component("BoplBody")?;
    let f = &c.fields;
    Some(BodyDef {
        mass: fx_or(f, "mass", Fix::ONE),
        bounciness: fx(f, "bounciness"),
        friction: fx(f, "friction"),
        dynamic_friction: fx(f, "dynamicFriction"),
        moment_of_inertia: fx_or(f, "momentOfInertia", Fix::ONE),
        gravity_scale: fx(f, "gravityScale"),
        air_friction01: fx_or(f, "airFriction01", Fix::ONE),
        rotational_drag01: fx_or(f, "rotationalDrag01", Fix::ONE),
        start_velocity: f.get("StartVelocity").and_then(vec2).unwrap_or_default(),
        start_angular_velocity: fx(f, "StartAngularVelocity"),
    })
}

fn collider(o: &Object) -> Option<(ColliderDef, i32, bool)> {
    let s = o.shape.as_ref()?;
    let (kind, ext, radius, min_ext) = match &s.kind {
        DetShapeKind::Box {
            extents,
            minimum_extents,
        } => (
            ShapeKind::Box,
            vraw(extents),
            Fix::ZERO,
            Fix(*minimum_extents),
        ),
        DetShapeKind::Circle { radius } => (ShapeKind::Circle, Vec2::ZERO, Fix(*radius), Fix::ZERO),
        DetShapeKind::RoundedRect { extents, radius } => (
            ShapeKind::RoundedRect,
            vraw(extents),
            Fix(*radius),
            Fix::ZERO,
        ),
    };
    Some((
        ColliderDef {
            kind,
            start_extents: ext,
            start_radius: radius,
            minimum_extents: min_ext,
            min_scale: Fix(s.min_scale),
            max_scale: Fix(s.max_scale),
            manual_init: s.manually_call_init,
            body: body_def(o),
            initialized: false,
        },
        s.hierarchy_number as i32,
        s.enabled,
    ))
}

fn visual(o: &Object, sprites: &BTreeMap<String, SpriteId>) -> VisualDef {
    VisualDef {
        object: o.id.clone(),
        sprite: o
            .sprite
            .as_ref()
            .and_then(|s| s.sprite.as_ref())
            .and_then(|s| sprites.get(s).copied()),
        sorting_layer: o
            .sprite
            .as_ref()
            .map_or(0, |s| s.sorting_layer_index as i32),
        sorting_order: o.sprite.as_ref().map_or(0, |s| s.sorting_order),
    }
}

fn prefab_object(o: &Object, sprites: &BTreeMap<String, SpriteId>) -> PrefabObject {
    let col = collider(o);
    PrefabObject {
        name: o.name.clone(),
        parent: o.parent,
        active: o.active,
        layer: o.layer as u8,
        tag: o.tag.clone(),
        fix: o.fix.as_ref().map(|_| transform(o)),
        collider: col.map(|c| c.0),
        collider_hier: col.map(|c| c.1),
        collider_enabled: col.is_some_and(|c| c.2),
        components: o
            .components
            .iter()
            .map(|c| PrefabComponent {
                class: c.script.clone().unwrap_or_else(|| c.kind.clone()),
                id: c.id.clone(),
                enabled: c.enabled.unwrap_or(true),
                hierarchy: c
                    .fields
                    .get("hierarchyNumber")
                    .and_then(J::as_i64)
                    .map(|h| h as i32),
                fields: value(&c.fields),
            })
            .collect(),
        visual: visual(o, sprites),
    }
}

// -- content ---------------------------------------------------------------------------

/// Builds the simulation content from loaded game data (for hosts that
/// already hold a [`GameData`]).
pub fn build_content(g: &GameData) -> Result<Content> {
    let mut c = Content::default();
    for id in g.sprites.keys() {
        let sid = SpriteId(c.sprites.len() as u32);
        c.sprites.push(id.clone());
        c.sprite_index.insert(id.clone(), sid);
    }
    // Frame animations.
    let mut anim_ids = BTreeMap::new();
    for (id, a) in &g.assets {
        if a.script != "AnimationData" {
            continue;
        }
        let f = &a.fields;
        let anims = f["animations"]
            .as_array()
            .map(|v| v.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|an| Animation {
                name: an["name"].as_str().unwrap_or("").to_string(),
                frames: an["animation"]
                    .as_array()
                    .map(|v| v.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|fr| Frame {
                        sprite: fr["sprite"]["$ref"]
                            .as_str()
                            .map_or(SpriteId::NONE, |s| c.sprite(s)),
                        duration: fr["frameDuration"].as_i64().unwrap_or(0) as i32,
                    })
                    .collect(),
            })
            .collect();
        let speed = f["velocityAnimations"]
            .as_array()
            .map(|v| v.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|an| SpeedAnimation {
                name: an["name"].as_str().unwrap_or("").to_string(),
                frames: an["animation"]
                    .as_array()
                    .map(|v| v.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|fr| SpeedFrame {
                        from: fx(fr, "speedFrom"),
                        to: fx(fr, "speedTo"),
                        sprite: fr["sprite"]["$ref"]
                            .as_str()
                            .map_or(SpriteId::NONE, |s| c.sprite(s)),
                    })
                    .collect(),
            })
            .collect();
        anim_ids.insert(id.clone(), c.anim_data.len() as u16);
        c.anim_data.push(AnimationData {
            id: id.clone(),
            name: a.name.clone(),
            animations: anims,
            speed_animations: speed,
            default_frame_duration: int(f, "defaultFrameDuration") as i32,
        });
    }
    // Prefabs (generic).
    for root in g.prefabs.roots.keys() {
        let p = g.prefab(root).with_context(|| format!("prefab {root}"))?;
        let pf = Prefab {
            id: p.id.clone(),
            name: p.name.clone(),
            objects: p
                .objects
                .iter()
                .map(|o| prefab_object(o, &c.sprite_index))
                .collect(),
        };
        c.prefab_index.insert(p.id.clone(), c.prefabs.len());
        c.prefabs.push(pf);
    }
    // Every object/component id resolves to its prefab.
    for (obj, root) in &g.prefabs.objects {
        if let Some(&i) = c.prefab_index.get(root) {
            c.prefab_index.entry(obj.clone()).or_insert(i);
        }
    }
    // Layer matrix.
    let m = g.index.settings.physics2d["m_LayerCollisionMatrix"]
        .as_array()
        .context("no layer collision matrix")?;
    for (i, w) in m.iter().enumerate().take(32) {
        c.layer_matrix[i] = w.as_u64().unwrap_or(0) as u32;
    }
    c.player = Some(player_prefab(g, &c, &anim_ids)?);
    let (pk, pi) = pickup(g, &c)?;
    c.pickup = Some(pk);
    c.pickup_prefab = pi;
    c.abilities = abilities(g, &c);
    Ok(c)
}

fn find_component<'a>(
    p: &'a format::Prefab,
    class: &str,
) -> Option<(&'a Object, &'a format::Component)> {
    p.objects
        .iter()
        .find_map(|o| o.component(class).map(|c| (o, c)))
}

fn effect_count(g: &GameData, id: Option<&str>) -> (i32, Vec2) {
    let Some(id) = id else {
        return (0, Vec2::ZERO);
    };
    match g.prefab(id) {
        Ok(p) => {
            let n = p
                .objects
                .iter()
                .flat_map(|o| o.components.iter())
                .filter(|c| c.fields.get("hierarchyNumber").is_some())
                .count() as i32;
            let off = p
                .objects
                .first()
                .map_or(Vec2::ZERO, |o| transform(o).position);
            (n, off)
        }
        Err(_) => (0, Vec2::ZERO),
    }
}

fn player_prefab(
    g: &GameData,
    c: &Content,
    anim_ids: &BTreeMap<String, u16>,
) -> Result<PlayerPrefab> {
    let id = g.player.prefab.as_ref().context("no player prefab")?;
    let p = g.prefab(id)?;
    let (root, phys) = find_component(&p, "PlayerPhysics").context("PlayerPhysics")?;
    let pf = &phys.fields;
    let (_, ctrl) = find_component(&p, "SlimeController").context("SlimeController")?;
    let cf = &ctrl.fields;
    let (_, col) = find_component(&p, "PlayerCollision").context("PlayerCollision")?;
    let (_, bounds) = find_component(&p, "DestroyIfOutsideSceneBounds").context("bounds")?;
    let (_, body) = find_component(&p, "PlayerBody").context("PlayerBody")?;
    let (sprite_obj, anim) = find_component(&p, "SpriteAnimator").context("SpriteAnimator")?;
    let (hurtbox, hhier, _) = collider(root).context("hurtbox")?;
    let bf = &bounds.fields;
    let rf = |k: &str| {
        bf.get(k)
            .and_then(|v| v["$ref"].as_str())
            .map(str::to_string)
    };
    let (down_n, down_off) = effect_count(g, rf("DownEffect").as_deref());
    let (up_n, _) = effect_count(g, rf("UpEffect").as_deref());
    let (x_n, _) = effect_count(g, rf("XEffect").as_deref());
    let death = col.fields["deathEffect"]["$ref"]
        .as_str()
        .map(str::to_string);
    let (death_n, _) = effect_count(g, death.as_deref());
    let anim_ref = cf["animData"]["$ref"].as_str().unwrap_or_default();
    Ok(PlayerPrefab {
        prefab: p.id.clone(),
        hier_physics: hier(pf),
        hier_controller: hier(cf),
        hier_collision: hier(&col.fields),
        hier_bounds: hier(bf),
        hier_body: hier(&body.fields),
        hier_hurtbox: hhier,
        hier_animator: hier(&anim.fields),
        max_speed: fx(pf, "maxSpeed"),
        jump_strength: fx(pf, "jumpStrength"),
        accel: fx(pf, "accel"),
        radius: fx(pf, "radius"),
        gravity_accel: fx(pf, "gravity_accel"),
        gravity_max_fall_speed: fx(pf, "gravity_maxFallSpeed"),
        gravity_modifier: fx(pf, "gravity_modifier"),
        jump_extra_x_strength: fx(pf, "jumpExtraXStrength"),
        jump_kept_momentum: fx(pf, "jumpKeptMomentum"),
        jump_normal_scale_factor: fx(pf, "jumpNormalScaleFactor"),
        extra_jump_teleport_multiplier: fx(pf, "extraJumpTeleportMultiplier"),
        air_accel: fx(pf, "airAccel"),
        inverse_mass01: fx(pf, "inverseMass01"),
        apply_gravity: flag(pf, "ApplyGravity"),
        sync_rotation_to_platform: flag(pf, "SyncRotationToPlatform"),
        cannot_be_ungrounded_by_ropes: flag(pf, "cannotBeUngroundedByRopes"),
        rope_circle_cast_radius: fx(cf, "RopeCircleCastRadius"),
        extra_swing_force: fx(cf, "extraSwingForce"),
        push_overlapping_players_str: fx(cf, "pushOverlappingPlayersStr"),
        scale_diff_before_nom_nom: fx(cf, "scaleDiffBeforeNomNom"),
        scale_increase_on_nom: fx(cf, "scaleIncreaseOnNom"),
        anim_data: *anim_ids.get(anim_ref).context("player animation data")?,
        is_squishable: flag(&col.fields, "isSquishable"),
        platform_squish_point_offset: col
            .fields
            .get("platformSquishPointOffset")
            .and_then(vec2)
            .unwrap_or_default(),
        rope_player_mass01: fx(&body.fields, "RopePlayerMass01"),
        ignore_gravity_when_roped: flag(&body.fields, "ignoreGravityWhenRoped"),
        destroy_if_too_far_x: flag(bf, "DestroyIfTooFarX"),
        destroy_if_too_far_y: flag(bf, "DestroyIfTooFarY"),
        do_not_destroy_up: flag(bf, "DoNotDestroyUp"),
        death_effect_updatables: death_n,
        down_effect_updatables: down_n,
        up_effect_updatables: up_n,
        x_effect_updatables: x_n,
        down_effect_offset: down_off,
        hurtbox,
        layer: root.layer as u8,
        visual: visual(sprite_obj, &c.sprite_index),
    })
}

fn pickup(g: &GameData, c: &Content) -> Result<(PickupDef, Option<usize>)> {
    let mut d = PickupDef::default();
    let Some(id) = g.player.ability_pickup.as_ref() else {
        return Ok((d, None));
    };
    let p = g.prefab(id)?;
    if let Some((o, comp)) = find_component(&p, "DynamicAbilityPickup") {
        let f = &comp.fields;
        d.up_force = fx_or(f, "UpForce", d.up_force);
        d.wobble_speed = fx_or(f, "wobbleSpeed", d.wobble_speed);
        d.wobble_amount = fx_or(f, "wobbleAmount", d.wobble_amount);
        d.launch_force = fx_or(f, "launchForce", d.launch_force);
        d.disabled_seconds = fx_or(f, "disabledSeconds", d.disabled_seconds);
        d.swap_period = fx_or(f, "SwapAbilityPeriod", d.swap_period);
        d.vibration_str = fx_or(f, "vibrationStr", d.vibration_str);
        d.vibration_duration = fx_or(f, "vibrationDuration", d.vibration_duration);
        d.vibrate_timing_offset = fx_or(f, "vibrateTimingOffset", d.vibrate_timing_offset);
        d.gravity = fx_or(f, "gravity", d.gravity);
        d.hier_pickup = hier(f);
        if let Some(b) = o.component("BoplBody") {
            d.hier_body = hier(&b.fields);
        }
        if let Some((col, h, _)) = collider(o) {
            d.collider = col;
            d.hier_circle = h;
        }
        d.layer = o.layer as u8;
    }
    d.updatables = p
        .objects
        .iter()
        .flat_map(|o| o.components.iter())
        .filter(|c| c.fields.get("hierarchyNumber").is_some())
        .count() as i32;
    Ok((d, c.prefab_index.get(&p.id).copied()))
}

fn abilities(g: &GameData, c: &Content) -> Vec<AbilityDef> {
    g.abilities
        .full
        .iter()
        .map(|e| {
            let f = &e.fields;
            let instant = e.kind.as_deref() == Some("InstantAbility");
            let scripts: Vec<&str> = e.scripts.iter().map(String::as_str).collect();
            AbilityDef {
                index: e.index,
                name: e.name.clone(),
                offensive: e.offensive,
                kind: if instant {
                    AbilityKind::Instant
                } else {
                    AbilityKind::Object
                },
                cooldown: if instant {
                    fx(f, "cooldown")
                } else {
                    fx(f, "Cooldown")
                },
                only_castable_on_ground: flag(f, "OnlyCastableOnGround"),
                kill_players_on_contact: flag(f, "KillPlayersOnContact"),
                is_pushable: flag(f, "isPushable"),
                cooldown_stopped_during_time_stop: flag(f, "cooldownStoppedDuringTimeStop"),
                is_rope: flag(f, "isHookshot") || scripts.contains(&"HookshotInstant"),
                icon: e.sprite.as_deref().map_or(SpriteId::NONE, |s| c.sprite(s)),
                prefab: e
                    .prefab
                    .as_ref()
                    .and_then(|p| c.prefab_index.get(p).copied()),
                fields: value(f),
            }
        })
        .collect()
}

// -- levels -------------------------------------------------------------------------------

fn phys_params(f: &J) -> PhysParams {
    let d = PhysParams::default();
    PhysParams {
        gravity: fx_or(f, "gravityAccel", d.gravity),
        rope_gravity: fx_or(f, "ropeGravity", d.rope_gravity),
        rope_iterations: f
            .get("ropeRelaxationIterations")
            .and_then(J::as_i64)
            .unwrap_or(7) as i32,
        rope_player_mass01: fx_or(f, "ropePlayerMass01", d.rope_player_mass01),
        rope_segment_radius: fx_or(f, "ropeSegmentRadius", d.rope_segment_radius),
        player_gravity: fx_or(f, "playerGravity", d.player_gravity),
        platform_stretchiness: fx_or(f, "platformStretchiness", d.platform_stretchiness),
        max_stretch_length: fx_or(f, "maxStretchLength", d.max_stretch_length),
        grounded_player_rope_str: fx_or(f, "groundedPlayerRopeStr", d.grounded_player_rope_str),
        max_beam_distance: fx_or(f, "maxBeamDistance", d.max_beam_distance),
        beam_width: fx_or(f, "beamWidth", d.beam_width),
        beam_parent_offset: fx_or(f, "beamParentOffset", d.beam_parent_offset),
        cross_beam_offset: fx_or(f, "crossBeamOffset", d.cross_beam_offset),
        beam_platform_push_force: fx_or(f, "beamPlatformPushForce", d.beam_platform_push_force),
        beam_black_hole_push_force: fx_or(
            f,
            "beamBlackHolePushForce",
            d.beam_black_hole_push_force,
        ),
        beam_rigid_body_push_force: fx_or(
            f,
            "beamRigidBodyPushFOrce",
            d.beam_rigid_body_push_force,
        ),
        beam_hit_mask: f["beamHitMask"]["m_Bits"]
            .as_u64()
            .unwrap_or(d.beam_hit_mask as u64) as u32,
    }
}

fn spring(o: &Object) -> Option<SpringDef> {
    let c = o.component("AnimateVelocity")?;
    let f = &c.fields;
    Some(SpringDef {
        hier: hier(f),
        enabled: c.enabled.unwrap_or(true),
        home_is_movable: flag(f, "HomeIsMovable"),
        speed: fx(f, "speed"),
        mu: fx(f, "mu"),
        rotation_speed: fx(f, "rotationSpeed"),
        rotation_mu: fx(f, "rotationMu"),
        interpolate_rotation_home: flag(f, "interpolateRotationHome"),
        rotation_resistence: fx_or(f, "rotationResistence", Fix::ONE),
        sudden_death_curve: Arc::new(
            f.get("suddenDeathYSpeed")
                .and_then(curve)
                .unwrap_or_default(),
        ),
        sudden_death_speed: fx(f, "suddenDeathSpeed"),
        sudden_death_acceleration: fx(f, "SuddenDeathAcceleration"),
        max_random_sudden_death_delay: fx(f, "MaxRandomSuddenDeathDelay"),
        home_rotation: fx(f, "homeRotation"),
    })
}

fn platform(o: &Object, sprites: &BTreeMap<String, SpriteId>) -> Result<PlatformDef> {
    let (col, hcol, _) =
        collider(o).with_context(|| format!("platform {} has no collider", o.name))?;
    let sticky = o.component("StickyRoundedRectangle").context("sticky")?;
    let bb = o.component("BoplBody");
    let shake = o.component("ShakablePlatform");
    Ok(PlatformDef {
        name: o.name.clone(),
        tag: o.tag.clone(),
        layer: o.layer as u8,
        transform: transform(o),
        collider: col,
        hier_collider: hcol,
        body: body_def(o).unwrap_or_default(),
        hier_body: bb.map_or(0, |b| hier(&b.fields)),
        hier_sticky: hier(&sticky.fields),
        platform_type: int(&sticky.fields, "platformType") as i32,
        spring: spring(o),
        hier_shake: shake.map(|s| hier(&s.fields)),
        shake_strength: shake.map_or(Fix::ONE, |s| fx_or(&s.fields, "basicShakeStr", Fix::ONE)),
        grow_on_start: o
            .component("GrowOnStart")
            .map(|g| (hier(&g.fields), fx_or(&g.fields, "scaleUp", Fix::ONE))),
        orbit: o.component("VectorFieldPlatform").map(|v| {
            let f = &v.fields;
            OrbitDef {
                hier: hier(f),
                delay: fx(f, "DelaySeconds"),
                orbit_speed: fx(f, "orbitSpeed"),
                expand_speed: fx(f, "expandSpeed"),
                center: f.get("centerPoint").and_then(vec2).unwrap_or_default(),
                normal_speed_friction: fx(f, "normalSpeedFriction"),
                dead_zone: fx(f, "DeadZoneDist"),
                orbit_accel: fx(f, "OrbitAccelerationMulitplier"),
                target_radius: fx(f, "targetRadius"),
            }
        }),
        path: o.component("AntiLockPlatform").map(|a| {
            let f = &a.fields;
            PathDef {
                hier: hier(f),
                points: f["OrbitPath"]
                    .as_array()
                    .map(|v| v.iter().filter_map(vec2).collect())
                    .unwrap_or_default(),
                force: fx(f, "OrbitForce"),
                delay: fx(f, "DelaySeconds"),
            }
        }),
        visual: visual(o, sprites),
    })
}

/// Depth-first pre-order over the hierarchy under `root` (engine.md §5.2).
fn dfs(objs: &[Object], root: usize, out: &mut Vec<usize>) {
    out.push(root);
    for &c in &objs[root].children {
        dfs(objs, c, out);
    }
}

/// Converts an imported level to sim data (the tutorial scene, build index
/// [`bopl_import::load::TUTORIAL`], through [`tutorial_level`]).
pub fn level(content: &Content, lv: &format::Level) -> Result<LevelData> {
    if lv.build_index == bopl_import::load::TUTORIAL {
        return tutorial_level(content, lv);
    }
    let s = &lv.session;
    let objs = &lv.objects;
    // Platforms: StickyRoundedRectangles under the root of `Level`, DFS order.
    let level_ref = s["Level"]["$ref"]
        .as_str()
        .context("session without Level")?;
    let mut li = objs
        .iter()
        .position(|o| o.id == level_ref)
        .context("Level object")?;
    while let Some(p) = objs[li].parent {
        li = p;
    }
    let mut order = Vec::new();
    dfs(objs, li, &mut order);
    let mut platforms = Vec::new();
    let mut platform_ids = Vec::new();
    for &i in &order {
        let o = &objs[i];
        if o.active_in_hierarchy && o.component("StickyRoundedRectangle").is_some() {
            platforms.push(platform(o, &content.sprite_index)?);
            platform_ids.push(i);
        }
    }
    let mut statics = Vec::new();
    for &i in &lv.shapes {
        if platform_ids.contains(&i) || !objs[i].active_in_hierarchy {
            continue;
        }
        let o = &objs[i];
        if let Some((col, h, en)) = collider(o) {
            statics.push(StaticDef {
                name: o.name.clone(),
                tag: o.tag.clone(),
                layer: o.layer as u8,
                transform: transform(o),
                collider: col,
                hier: h,
                enabled: en,
            });
        }
    }
    let mut spawners = Vec::new();
    for o in objs {
        if let Some(c) = o.component("AbilitySpawner")
            && o.active_in_hierarchy
        {
            spawners.push(SpawnerDef {
                hier: hier(&c.fields),
                position: transform(o).position,
            });
        }
    }
    if lv.team_spawns.len() < 4 {
        bail!(
            "level {} has {} team spawns",
            lv.build_index,
            lv.team_spawns.len()
        );
    }
    let team_spawns = [
        vraw(&lv.team_spawns[0]),
        vraw(&lv.team_spawns[1]),
        vraw(&lv.team_spawns[2]),
        vraw(&lv.team_spawns[3]),
    ];
    let world = match lv.level_type {
        1 => LevelWorld::Snow,
        2 => LevelWorld::Space,
        _ => LevelWorld::Grass,
    };
    let session = SessionDef {
        hier: hier(s),
        level_animation_radius: fx(s, "levelAnimationRadius"),
        max_random_delay: fx(s, "MaxRandomDelay"),
        time_before_two: fx(s, "timeBeforeTwo"),
        time_before_go: fx(s, "timeBeforeGO"),
        level_animation_time: fx_or(s, "LevelAnimationTime", Fix::TWO),
        time_before_sudden_death: fx(s, "TimeBeforeSuddenDeath"),
        delay_before_next_level_input: fx(s, "DelayBeforeNextLevelInput"),
        start_progress: fx(s, "startProgressOfAnimateInGrounds"),
        block_in_delay: fx(s, "blockAnimateInDelay_constantDelayFactor"),
        block_out_delay: fx(s, "blockAnimateOutDelay_constantDelayFactor"),
        animate_in: Arc::new(s.get("AnimateInOut").and_then(curve).unwrap_or_default()),
        animate_out: Arc::new(s.get("AnimateOut").and_then(curve).unwrap_or_default()),
    };
    let mut ld = LevelData {
        build_index: lv.build_index,
        name: lv.name.clone(),
        world,
        team_spawns,
        teammate_spacing: Fix(lv.teammate_spawn_spacing),
        extra_spawns: Vec::new(),
        session,
        physics: phys_params(&lv.physics),
        beam_width_anim: Arc::new(
            lv.physics
                .get("beamWidthAnim")
                .and_then(curve)
                .unwrap_or_default(),
        ),
        platforms,
        statics,
        spawners,
        tutorial: None,
    };
    ld.extra_spawns = spawns::generate(&ld, 8);
    Ok(ld)
}

/// The tutorial scene (docs/spec/tutorial.md) as sim data: its platforms
/// (every active sticky platform, roots in file order, depth first), the
/// water box, and the `TutorialGameHandler` with its arrows, pickups,
/// dummies and the dummies' grounds.
pub fn tutorial_level(content: &Content, lv: &format::Level) -> Result<LevelData> {
    let objs = &lv.objects;
    let s = &lv.session;
    // Component id -> owning object.
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, o) in objs.iter().enumerate() {
        for c in &o.components {
            owner.insert(c.id.as_str(), i);
        }
        owner.insert(o.id.as_str(), i);
    }
    let by_ref = |j: &J| -> Option<usize> { owner.get(j["$ref"].as_str()?).copied() };
    let refs = |j: &J| -> Vec<usize> {
        j.as_array()
            .map(|a| a.iter().filter_map(by_ref).collect())
            .unwrap_or_default()
    };
    // Platforms.
    let mut order = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        if o.parent.is_none() {
            dfs(objs, i, &mut order);
        }
    }
    let mut platforms = Vec::new();
    let mut platform_ids = Vec::new();
    for &i in &order {
        let o = &objs[i];
        if o.active_in_hierarchy && o.component("StickyRoundedRectangle").is_some() {
            platforms.push(platform(o, &content.sprite_index)?);
            platform_ids.push(i);
        }
    }
    let obj_def = |i: usize, class: &str| -> Result<TutorialObjDef> {
        let o = &objs[i];
        let (col, hcol, _) =
            collider(o).with_context(|| format!("tutorial object {} has no collider", o.name))?;
        Ok(TutorialObjDef {
            name: o.name.clone(),
            object: i,
            layer: o.layer as u8,
            transform: transform(o),
            collider: col,
            hier_collider: hcol,
            hier: o.component(class).map_or(0, |c| hier(&c.fields)),
            active: o.active,
        })
    };
    let arrows = refs(&s["arrows"])
        .into_iter()
        .map(|i| obj_def(i, "TutorialArrow"))
        .collect::<Result<Vec<_>>>()?;
    let mut pickups = Vec::new();
    for key in ["bowPickup", "grenadePickup", "dashPickup"] {
        let i = by_ref(&s[key]).with_context(|| format!("tutorial {key}"))?;
        let f = &objs[i]
            .component("TutorialArrowPickup")
            .context("TutorialArrowPickup")?
            .fields;
        pickups.push(TutorialPickupDef {
            obj: obj_def(i, "TutorialArrowPickup")?,
            slot: int(f, "abilityToUnlock").clamp(0, 2) as usize,
            hover_speed: fx(f, "hoverSpeed"),
            hover_amplitude: fx(f, "hoverAmplitude"),
        });
    }
    let ground_objs = refs(&s["groundsWithDummies"]);
    let mut grounds = Vec::new();
    for &gi in &ground_objs {
        let o = &objs[gi];
        let f = &o
            .component("AnimateGround")
            .context("AnimateGround")?
            .fields;
        grounds.push(AnimateGroundDef {
            platform: platform_ids
                .iter()
                .position(|&p| p == gi)
                .with_context(|| format!("ground {} is no platform", o.name))?,
            keys: f["PathKeys"]
                .as_array()
                .map(|a| a.iter().filter_map(vec2).collect())
                .unwrap_or_default(),
            curve: Arc::new(f.get("PositionCurve").and_then(curve).unwrap_or_default()),
            delay: fx(f, "DelaySeconds"),
            hier: hier(f),
        });
    }
    let mut dummies = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        let Some(c) = o.component("TutorialTargetDummy") else {
            continue;
        };
        let f = &c.fields;
        let g =
            by_ref(&f["animateGround"]).and_then(|gi| ground_objs.iter().position(|&x| x == gi));
        dummies.push(TutorialDummyDef {
            obj: obj_def(i, "TutorialTargetDummy")?,
            ground: g.context("dummy without a ground")?,
            mask: f["layerMask"]["m_Bits"].as_u64().unwrap_or(0) as u32,
            unlock_grenade: flag(f, "unlockGrenadeOnDeath"),
            unlock_dash: flag(f, "unlockDashOnDeath"),
        });
    }
    // The player's abilities: the scene's ability objects by slot, as
    // ability-list indices (by name), and their (locked) cooldown.
    let ctrl = objs
        .iter()
        .find_map(|o| o.component("SlimeController"))
        .context("tutorial player")?;
    let mut abilities = [4usize, 3, 2];
    let mut locked = Fix::from_int(100_000);
    for (k, ai) in refs(&ctrl.fields["abilities"])
        .into_iter()
        .take(3)
        .enumerate()
    {
        let name = objs[ai].name.to_lowercase();
        if let Some(a) = content
            .abilities
            .iter()
            .find(|a| a.name.to_lowercase() == name)
        {
            abilities[k] = a.index;
        }
        if let Some(c) = objs[ai].component("Ability") {
            locked = fx_or(&c.fields, "Cooldown", locked);
        }
    }
    let cds: Vec<Fix> = s["cooldowns"]
        .as_array()
        .map(|a| a.iter().filter_map(fixv).collect())
        .unwrap_or_default();
    let cooldown = |k: usize| cds.get(k).copied().unwrap_or(Fix::ONE);
    let tutorial_objs: Vec<usize> = arrows
        .iter()
        .map(|a| a.object)
        .chain(pickups.iter().map(|p| p.obj.object))
        .chain(dummies.iter().map(|d| d.obj.object))
        .collect();
    let mut statics = Vec::new();
    for &i in &lv.shapes {
        let o = &objs[i];
        if platform_ids.contains(&i)
            || tutorial_objs.contains(&i)
            || !o.active_in_hierarchy
            || o.component("PlayerPhysics").is_some()
        {
            continue;
        }
        if let Some((col, h, en)) = collider(o) {
            statics.push(StaticDef {
                name: o.name.clone(),
                tag: o.tag.clone(),
                layer: o.layer as u8,
                transform: transform(o),
                collider: col,
                hier: h,
                enabled: en,
            });
        }
    }
    let spawn = vraw(&lv.team_spawns[0]);
    let tutorial = TutorialDef {
        hier: hier(s),
        player_spawn: s.get("playerSpawn").and_then(vec2).unwrap_or(spawn),
        abilities,
        locked_cooldown: locked,
        cooldowns: [cooldown(0), cooldown(1), cooldown(2)],
        jingle_delay: fx(s, "DelayOfYouWinJingle"),
        exit_delay: fx_or(s, "DelayAfterCompletedTutorial", Fix::ONE),
        arrows,
        pickups,
        dummies,
        grounds,
    };
    Ok(LevelData {
        build_index: lv.build_index,
        name: lv.name.clone(),
        world: LevelWorld::Grass,
        team_spawns: [spawn; 4],
        teammate_spacing: Fix::ZERO,
        extra_spawns: Vec::new(),
        session: SessionDef::default(),
        physics: phys_params(&lv.physics),
        beam_width_anim: Arc::new(
            lv.physics
                .get("beamWidthAnim")
                .and_then(curve)
                .unwrap_or_default(),
        ),
        platforms,
        statics,
        spawners: Vec::new(),
        tutorial: Some(Arc::new(tutorial)),
    })
}

/// Loads and converts a level by build index.
pub fn load_level(l: &Loaded, build_index: usize) -> Result<Arc<LevelData>> {
    let lv = l.game.level(build_index)?;
    Ok(Arc::new(level(&l.content, &lv)?))
}
