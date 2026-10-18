//! Mecanim animation clips and controllers (`animations.json`).
//!
//! A built `AnimationClip` stores its curves in `m_MuscleClip.m_Clip`:
//!
//! * a *streamed* clip: `u32` words forming frames `f32 time, u32 key
//!   count, keys (u32 curve index, f32 a, b, c, d)`. A key starts a cubic
//!   piece `((a*dt + b)*dt + c)*dt + d` of its curve at the frame time,
//!   valid until the curve's next key. The first frame has time `-f32::MAX`
//!   (constant lead-in), the last `+inf` (no keys);
//! * a *dense* clip: `frame count x curve count` samples (frame-major) at a
//!   sample rate from a begin time, linearly interpolated;
//! * a *constant* clip: one value per curve.
//!
//! Curve indices count streamed curves first (`curveCount` float curves,
//! then `discreteCurveCount` object-reference curves), then dense, then
//! constant.
//! `m_ClipBindingConstant.genericBindings` assigns them, in order, to
//! targets: a Transform binding (class 4) with attribute 1 (position), 3
//! (scale) or 4 (euler) takes 3 curves, attribute 2 (quaternion) 4,
//! anything else 1 (attribute = CRC32 of the property path). `path` is the
//! CRC32 of the target's transform path below the Animator.
//!
//! Controllers: `m_Controller` holds layers, state machines with states
//! (name hash, speed, loop, blend trees whose leaves name clips by index
//! into `m_AnimationClips`), and `m_TOS` maps hashes to names.

use std::collections::{BTreeMap, HashMap};

use unity_assets::{AssetDir, AssetFile, Value};

use crate::format::{self, ObjId, path_hash};
use crate::scene::{obj_id, ref_id};

/// Property-path names by hash for one target class (engine or script).
struct Names<'a> {
    dir: &'a AssetDir,
    cache: HashMap<String, HashMap<u32, String>>,
}

const COMMON: &[&str] = &[
    "m_IsActive",
    "m_Enabled",
    "m_Color.r",
    "m_Color.g",
    "m_Color.b",
    "m_Color.a",
    "m_Sprite",
    "m_Alpha",
];

/// Dotted field paths of a type tree (arrays skipped).
fn paths(raw: &[unity_assets::typetree::RawNode]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<(u8, String, bool)> = Vec::new(); // level, path, in array
    for n in raw.iter().skip(1) {
        while stack.last().is_some_and(|s| s.0 >= n.level) {
            stack.pop();
        }
        let (parent, in_array) = stack
            .last()
            .map(|s| (s.1.clone(), s.2))
            .unwrap_or((String::new(), false));
        let array = in_array || n.type_name == "Array";
        let p = if parent.is_empty() {
            n.name.clone()
        } else {
            format!("{parent}.{}", n.name)
        };
        if !array {
            out.push(p.clone());
        }
        stack.push((n.level, p, array));
    }
    out
}

impl Names<'_> {
    fn lookup(
        &mut self,
        class_id: i32,
        script: Option<&unity_assets::ScriptKey>,
        hash: u32,
    ) -> String {
        let key = match script {
            Some(k) => format!("s:{}", k.full_name()),
            None => format!("e:{class_id}"),
        };
        let reg = self.dir.registry();
        let map = self.cache.entry(key).or_insert_with(|| {
            let mut m: HashMap<u32, String> = COMMON
                .iter()
                .map(|s| (path_hash(s), s.to_string()))
                .collect();
            let tree = match script {
                Some(k) => reg.script(k),
                None => reg.engine(class_id),
            };
            if let Some(t) = tree {
                for p in paths(t.raw()) {
                    m.entry(path_hash(&p)).or_insert(p);
                }
            }
            m
        });
        map.get(&hash)
            .cloned()
            .unwrap_or_else(|| format!("#{hash}"))
    }
}

fn u32s(v: &Value) -> Vec<u32> {
    v.as_array()
        .iter()
        .map(|x| x.as_i64().map(|i| i as u32).unwrap_or(0))
        .collect()
}

fn f32s(v: &Value) -> Vec<f32> {
    v.as_array().iter().map(|x| x.f32()).collect()
}

/// Curve index -> (binding index, component) for the generic bindings.
fn expand_bindings(bindings: &[Value]) -> Vec<(usize, &'static str)> {
    const XYZ: [&str; 3] = ["x", "y", "z"];
    const XYZW: [&str; 4] = ["x", "y", "z", "w"];
    let mut out = Vec::new();
    for (bi, b) in bindings.iter().enumerate() {
        let comps: &[&str] = match (b.get("typeID").i64(), b.get("attribute").i64()) {
            (4, 1 | 3 | 4) => &XYZ,
            (4, 2) => &XYZW,
            _ => &[""],
        };
        out.extend(comps.iter().map(|c| (bi, *c)));
    }
    out
}

/// Reads one `AnimationClip`.
pub fn clip(
    dir: &AssetDir,
    f: &AssetFile,
    v: &Value,
    names: &mut HashMap<String, HashMap<u32, String>>,
) -> format::AnimClip {
    let mut nm = Names {
        dir,
        cache: std::mem::take(names),
    };
    let mc = v.get("m_MuscleClip");
    let data = mc.path("m_Clip.data");
    let start = mc.get("m_StartTime").f32();
    let stop = mc.get("m_StopTime").f32();
    let bc = v.get("m_ClipBindingConstant");
    let bindings = bc.get("genericBindings").as_array();
    let slots = expand_bindings(bindings);
    let mut segs: Vec<Vec<[f32; 5]>> = vec![Vec::new(); slots.len()];
    // Streamed.
    let words = u32s(data.path("m_StreamedClip.data"));
    // Discrete (object-reference) curves follow the float curves of the
    // streamed block.
    let streamed = (data.path("m_StreamedClip.curveCount").i64().max(0)
        + data.path("m_StreamedClip.discreteCurveCount").i64().max(0)) as usize;
    let mut i = 0;
    while i + 2 <= words.len() {
        let t = f32::from_bits(words[i]);
        let n = words[i + 1] as usize;
        i += 2;
        for _ in 0..n {
            if i + 5 > words.len() {
                break;
            }
            let c = words[i] as usize;
            let k = [1, 2, 3, 4].map(|j| f32::from_bits(words[i + j]));
            i += 5;
            if t.is_finite()
                && let Some(s) = segs.get_mut(c)
            {
                // The -f32::MAX lead-in frame starts at the clip start.
                let t0 = if t <= -1e30 { start } else { t };
                s.push([t0, k[0], k[1], k[2], k[3]]);
            }
        }
    }
    // Dense.
    let dense = data.get("m_DenseClip");
    let dn = dense.get("m_CurveCount").i64().max(0) as usize;
    let frames = dense.get("m_FrameCount").i64().max(0) as usize;
    let rate = dense.get("m_SampleRate").f32().max(1e-3);
    let begin = dense.get("m_BeginTime").f32();
    let samples = f32s(dense.get("m_SampleArray"));
    for c in 0..dn {
        let Some(s) = segs.get_mut(streamed + c) else {
            break;
        };
        for fr in 0..frames {
            let Some(&a) = samples.get(fr * dn + c) else {
                break;
            };
            let b = samples.get((fr + 1) * dn + c).copied().unwrap_or(a);
            s.push([begin + fr as f32 / rate, 0.0, 0.0, (b - a) * rate, a]);
        }
    }
    // Constant.
    for (k, val) in f32s(data.path("m_ConstantClip.data"))
        .into_iter()
        .enumerate()
    {
        if let Some(s) = segs.get_mut(streamed + dn + k) {
            s.push([start, 0.0, 0.0, 0.0, val]);
        }
    }
    let mut curves = Vec::with_capacity(slots.len());
    for ((bi, comp), mut s) in slots.into_iter().zip(segs) {
        let b = &bindings[bi];
        let class_id = b.get("typeID").i64() as i32;
        let attr = b.get("attribute").i64() as u32;
        let script = b
            .get("script")
            .as_pptr()
            .filter(|p| !p.is_null())
            .and_then(|p| dir.deref(f, p).ok().flatten())
            .map(|(_, _, sv)| {
                unity_assets::ScriptKey::new(
                    sv.get("m_AssemblyName").str(),
                    sv.get("m_Namespace").str(),
                    sv.get("m_ClassName").str(),
                )
            });
        let property = if class_id == 4 && (1..=4).contains(&attr) {
            let base = ["position", "rotation", "scale", "euler"][attr as usize - 1];
            format!("{base}.{comp}")
        } else {
            nm.lookup(class_id, script.as_ref(), attr)
        };
        s.sort_by(|a, b| a[0].total_cmp(&b[0]));
        curves.push(format::AnimCurve {
            path: b.get("path").i64() as u32,
            class_id,
            script: script.map(|k| k.full_name()),
            property,
            pptr: b.get("isPPtrCurve").i64() != 0,
            segments: s,
        });
    }
    *names = nm.cache;
    format::AnimClip {
        name: v.get("m_Name").str().to_owned(),
        start,
        stop,
        sample_rate: v.get("m_SampleRate").f32(),
        looping: mc.get("m_LoopTime").bool(),
        curves,
        pptr: bc
            .get("pptrCurveMapping")
            .as_array()
            .iter()
            .map(|p| p.as_pptr().and_then(|p| ref_id(f, p)))
            .collect(),
        events: crate::scene::to_json(f, v.get("m_Events")),
    }
}

/// Reads one `AnimatorController`.
pub fn controller(f: &AssetFile, v: &Value) -> format::AnimController {
    let tos: BTreeMap<u32, String> = v
        .get("m_TOS")
        .as_array()
        .iter()
        .map(|p| {
            let a = p.as_array();
            let (k, s) = match a {
                [k, s] => (k.i64() as u32, s.str().to_owned()),
                _ => (
                    p.get("first").i64() as u32,
                    p.get("second").str().to_owned(),
                ),
            };
            (k, s)
        })
        .collect();
    let name = |h: u32| tos.get(&h).cloned().unwrap_or_else(|| format!("#{h}"));
    let clips: Vec<ObjId> = v
        .get("m_AnimationClips")
        .as_array()
        .iter()
        .filter_map(|p| p.as_pptr().and_then(|p| ref_id(f, p)))
        .collect();
    let c = v.get("m_Controller");
    let machines = c.get("m_StateMachineArray").as_array();
    let layers = c
        .get("m_LayerArray")
        .as_array()
        .iter()
        .map(|l| {
            let l = l.get("data");
            let sm = machines
                .get(l.get("m_StateMachineIndex").i64().max(0) as usize)
                .map(|m| m.get("data"));
            let n_states = sm.map_or(0, |m| m.get("m_StateConstantArray").as_array().len());
            let states = sm
                .map(|m| {
                    m.get("m_StateConstantArray")
                        .as_array()
                        .iter()
                        .map(|st| {
                            let st = st.get("data");
                            let clip = st
                                .get("m_BlendTreeConstantArray")
                                .as_array()
                                .iter()
                                .flat_map(|bt| bt.path("data.m_NodeArray").as_array())
                                .map(|n| n.get("data").get("m_ClipID").i64())
                                .find(|&id| (0..clips.len() as i64).contains(&id))
                                .map(|id| clips[id as usize].clone());
                            let speed_param = st.get("m_SpeedParamID").i64() as u32;
                            format::AnimState {
                                name: name(st.get("m_FullPathID").i64() as u32),
                                speed: st.get("m_Speed").f32(),
                                looping: st.get("m_Loop").bool(),
                                clip,
                                transitions: st.get("m_TransitionConstantArray").as_array().len(),
                                outgoing: transitions(
                                    st.get("m_TransitionConstantArray"),
                                    n_states,
                                    &name,
                                ),
                                cycle_offset: st.get("m_CycleOffset").f32(),
                                speed_parameter: (speed_param != 0).then(|| name(speed_param)),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            format::AnimLayer {
                name: name(l.get("m_Binding").i64() as u32),
                default_weight: l.get("m_DefaultWeight").f32(),
                default_state: sm.map_or(0, |m| m.get("m_DefaultState").i64().max(0) as usize),
                states,
                any_state_transitions: sm
                    .map(|m| {
                        transitions(m.get("m_AnyStateTransitionConstantArray"), n_states, &name)
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    format::AnimController {
        name: v.get("m_Name").str().to_owned(),
        clips,
        layers,
        parameters: parameters(c, &name),
    }
}

/// Animator parameters from `m_Values` (types and default-value slots) and
/// `m_DefaultValues`.
fn parameters(c: &Value, name: &dyn Fn(u32) -> String) -> Vec<format::AnimParameter> {
    let d = c.path("m_DefaultValues.data");
    c.path("m_Values.data.m_ValueArray")
        .as_array()
        .iter()
        .map(|p| {
            let index = p.get("m_Index").i64().max(0) as usize;
            let pick = |arr: &str| d.get(arr).as_array().get(index).cloned();
            let (kind, default) = match p.get("m_Type").i64() {
                1 => ("float", pick("m_FloatValues").map_or(0.0, |x| x.f32())),
                3 => ("int", pick("m_IntValues").map_or(0.0, |x| x.i64() as f32)),
                4 => (
                    "bool",
                    pick("m_BoolValues").map_or(0.0, |x| x.bool() as i32 as f32),
                ),
                9 => (
                    "trigger",
                    pick("m_BoolValues").map_or(0.0, |x| x.bool() as i32 as f32),
                ),
                _ => ("unknown", 0.0),
            };
            format::AnimParameter {
                name: name(p.get("m_ID").i64() as u32),
                kind: kind.to_owned(),
                default,
            }
        })
        .collect()
}

/// A `TransitionConstant` array (`[{data: ...}]`).
fn transitions(
    arr: &Value,
    n_states: usize,
    name: &dyn Fn(u32) -> String,
) -> Vec<format::AnimTransition> {
    arr.as_array()
        .iter()
        .map(|t| {
            let t = t.get("data");
            let dest = t.get("m_DestinationState").i64() as u32;
            format::AnimTransition {
                destination: ((dest as usize) < n_states).then_some(dest as usize),
                destination_raw: dest,
                duration: t.get("m_TransitionDuration").f32(),
                offset: t.get("m_TransitionOffset").f32(),
                exit_time: t.get("m_ExitTime").f32(),
                has_exit_time: t.get("m_HasExitTime").bool(),
                fixed_duration: t.get("m_HasFixedDuration").bool(),
                interruption_source: t.get("m_InterruptionSource").i64() as i32,
                ordered_interruption: t.get("m_OrderedInterruption").bool(),
                can_transition_to_self: t.get("m_CanTransitionToSelf").bool(),
                conditions: t
                    .get("m_ConditionConstantArray")
                    .as_array()
                    .iter()
                    .map(|c| {
                        let c = c.get("data");
                        let mode = match c.get("m_ConditionMode").i64() {
                            1 => "if".to_owned(),
                            2 => "if_not".to_owned(),
                            3 => "greater".to_owned(),
                            4 => "less".to_owned(),
                            5 => "exit_time".to_owned(),
                            6 => "equals".to_owned(),
                            7 => "not_equal".to_owned(),
                            n => format!("#{n}"),
                        };
                        format::AnimCondition {
                            mode,
                            parameter: name(c.get("m_EventID").i64() as u32),
                            threshold: c.get("m_EventThreshold").f32(),
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Every clip and controller of `files`; problems as warnings.
pub fn animations(
    dir: &AssetDir,
    files: &[std::sync::Arc<AssetFile>],
) -> (format::Animations, Vec<String>) {
    let mut out = format::Animations::default();
    let mut warnings = Vec::new();
    let mut names = HashMap::new();
    for f in files {
        for o in f.objects_of(74) {
            match dir.read(f, o.path_id) {
                Ok(v) => {
                    out.clips
                        .insert(obj_id(f, o.path_id), clip(dir, f, &v, &mut names));
                }
                Err(e) => warnings.push(format!("clip {}: {e}", obj_id(f, o.path_id))),
            }
        }
        for o in f.objects_of(91) {
            match dir.read(f, o.path_id) {
                Ok(v) => {
                    out.controllers
                        .insert(obj_id(f, o.path_id), controller(f, &v));
                }
                Err(e) => warnings.push(format!("controller {}: {e}", obj_id(f, o.path_id))),
            }
        }
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unity_assets::typetree::RawNode;

    #[test]
    fn field_paths_skip_arrays() {
        let raw = vec![
            RawNode::new(0, "Image", "Base", -1, 0),
            RawNode::new(1, "ColorRGBA", "m_Color", -1, 0),
            RawNode::new(2, "float", "r", 4, 0),
            RawNode::new(1, "vector", "list", -1, 0),
            RawNode::new(2, "Array", "Array", -1, 0),
            RawNode::new(3, "int", "size", 4, 0),
            RawNode::new(1, "float", "m_FillAmount", 4, 0),
        ];
        assert_eq!(
            paths(&raw),
            ["m_Color", "m_Color.r", "list", "m_FillAmount"]
        );
    }
}
