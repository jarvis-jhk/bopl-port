//! Dev tool: checks the import against the UnityPy reference dump
//! (`ref/dump/<file>.json`, from `scripts/dev/dump_assets.py`).
//!
//! For every component of every imported level, scene and prefab it compares
//! the raw `fields` with the dump's values of the same object (references
//! resolved through the dump's externals to `"<file>:<path id>"`, floats as
//! `f32`), and the typed `fix` / `shape` values with the dump's raw `Fix`
//! bits. MonoBehaviour headers are skipped (the dump's generated header is
//! misaligned at `m_Script`).
//!
//! Usage: `cargo run --release -p bopl-import --example validate_import --
//! [--data DIR] [--dump DIR]` (defaults `cache/import`, `ref/dump`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bopl_import::GameData;
use bopl_import::format::{DetShapeKind, Object};
use rayon::prelude::*;
use serde_json::Value as J;

/// Python's json writes bare `NaN`/`Infinity`; quote them so serde_json parses.
fn quote_non_finite(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut in_str, mut esc, mut last) = (0, false, false, 0);
    while i < b.len() {
        let c = b[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        let tok = ["-Infinity", "Infinity", "NaN"]
            .into_iter()
            .find(|t| b[i..].starts_with(t.as_bytes()));
        match tok {
            Some(t) => {
                out.push_str(&s[last..i]);
                out.push_str("null");
                i += t.len();
                last = i;
            }
            None => i += 1,
        }
    }
    out.push_str(&s[last..]);
    out
}

struct Dump {
    externals: Vec<String>,
    objects: J,
}

struct Dumps {
    dir: PathBuf,
    cache: Mutex<HashMap<String, std::sync::Arc<Option<Dump>>>>,
}

impl Dumps {
    fn get(&self, file: &str) -> std::sync::Arc<Option<Dump>> {
        if let Some(d) = self.cache.lock().unwrap().get(file) {
            return d.clone();
        }
        let d = std::fs::read_to_string(self.dir.join(format!("{file}.json")))
            .ok()
            .and_then(|s| serde_json::from_str::<J>(&quote_non_finite(&s)).ok())
            .map(|j| Dump {
                externals: j["externals"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|e| {
                                let p = e.as_str().unwrap_or("");
                                p.rsplit(['/', '\\']).next().unwrap_or(p).to_owned()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                objects: j["objects"].clone(),
            });
        let d = std::sync::Arc::new(d);
        self.cache
            .lock()
            .unwrap()
            .insert(file.to_owned(), d.clone());
        d
    }
}

#[derive(Default)]
struct Stats {
    compared: usize,
    equal: usize,
    mismatches: usize,
    no_ref: usize,
    fix_checked: usize,
    fix_bad: usize,
    examples: Vec<String>,
}

fn is_pptr(m: &serde_json::Map<String, J>) -> bool {
    m.len() == 2 && m.contains_key("m_FileID") && m.contains_key("m_PathID")
}

/// Compares our value `a` with the dump's `d` (of an object in `file`).
fn cmp(a: &J, d: &J, file: &str, ext: &[String], path: &str) -> Option<String> {
    let bad = |why: &str| {
        let (sa, sd) = (a.to_string(), d.to_string());
        Some(format!(
            "{path}: {why}: ours {} dump {}",
            &sa[..sa.len().min(60)],
            &sd[..sd.len().min(60)]
        ))
    };
    if let J::Object(dm) = d {
        if is_pptr(dm) {
            let fid = dm["m_FileID"].as_i64().unwrap_or(-1);
            let pid = dm["m_PathID"].as_i64().unwrap_or(0);
            let want = if pid == 0 {
                J::Null
            } else {
                let f = if fid == 0 {
                    Some(file.to_owned())
                } else {
                    ext.get(fid as usize - 1).cloned()
                };
                match f {
                    Some(f) => serde_json::json!({ "$ref": format!("{f}:{pid}") }),
                    None => return bad("bad file id"),
                }
            };
            return (a != &want).then(|| bad("reference")).flatten();
        }
        if let Some(n) = dm.get("$bytes").or(dm.get("$array")).and_then(J::as_u64) {
            let len = a.as_array().map_or(usize::MAX, Vec::len);
            return (len as u64 != n).then(|| bad("length")).flatten();
        }
    }
    match (a, d) {
        (J::Null, J::Null) => None,
        (J::Number(x), J::Number(y)) => {
            if let (Some(x), Some(y)) = (x.as_i64(), y.as_i64()) {
                return (x != y).then(|| bad("int")).flatten();
            }
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            (x as f32 != y as f32).then(|| bad("float")).flatten()
        }
        // Unity bools stored as UInt8 are numbers on our side.
        (J::Number(x), J::Bool(y)) => (x.as_i64() != Some(*y as i64))
            .then(|| bad("bool"))
            .flatten(),
        (J::Bool(x), J::Bool(y)) => (x != y).then(|| bad("bool")).flatten(),
        (J::String(x), J::String(y)) => (x != y).then(|| bad("string")).flatten(),
        (J::Array(x), J::Array(y)) => {
            if x.len() != y.len() {
                return bad("array length");
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| cmp(p, q, file, ext, &format!("{path}[{i}]")))
        }
        // UnityPy writes `pair`s as [first, second].
        (J::Object(m), J::Array(y)) if m.len() == 2 && y.len() == 2 && m.contains_key("first") => {
            cmp(&m["first"], &y[0], file, ext, &format!("{path}.first"))
                .or_else(|| cmp(&m["second"], &y[1], file, ext, &format!("{path}.second")))
        }
        (J::Object(m), J::Object(n)) => {
            for (k, v) in m {
                let Some(w) = n.get(k) else {
                    return bad(&format!("field {k} missing in dump"));
                };
                if let Some(e) = cmp(v, w, file, ext, &format!("{path}.{k}")) {
                    return Some(e);
                }
            }
            let extra: Vec<&String> = n.keys().filter(|k| !m.contains_key(*k)).collect();
            let header = ["m_GameObject", "m_Enabled", "m_Script", "m_Name"];
            let extra: Vec<_> = extra
                .into_iter()
                .filter(|k| !header.contains(&k.as_str()))
                .collect();
            (!extra.is_empty())
                .then(|| bad(&format!("fields {extra:?} missing on our side")))
                .flatten()
        }
        _ => bad("type"),
    }
}

fn fixv(j: &J) -> i64 {
    j["m_rawValue"].as_i64().unwrap_or(i64::MIN)
}

fn check_objects(objects: &[Object], dumps: &Dumps, st: &Mutex<Stats>) {
    let mut local = Stats::default();
    for o in objects {
        for c in &o.components {
            if c.fields.is_null() {
                continue;
            }
            let Some((file, pid)) = c.id.rsplit_once(':') else {
                continue;
            };
            let d = dumps.get(file);
            let Some(dump) = d.as_ref() else {
                local.no_ref += 1;
                continue;
            };
            let data = &dump.objects[pid]["data"];
            if data.is_null() {
                local.no_ref += 1;
                continue;
            }
            local.compared += 1;
            match cmp(&c.fields, data, file, &dump.externals, "") {
                None => local.equal += 1,
                Some(e) => {
                    local.mismatches += 1;
                    if local.examples.len() < 5 {
                        local.examples.push(format!(
                            "{} {}: {e}",
                            c.id,
                            c.script.as_deref().unwrap_or(&c.kind)
                        ));
                    }
                }
            }
            // Typed values against the dump's raw bits.
            let mut fix_bad = false;
            if let Some(f) = o.fix.as_ref().filter(|f| f.component == c.id) {
                local.fix_checked += 1;
                fix_bad |= f.position
                    != [fixv(&data["_position"]["x"]), fixv(&data["_position"]["y"])]
                    || f.rotation != fixv(&data["_rotation"]);
            }
            if let Some(s) = o.shape.as_ref().filter(|s| s.component == c.id) {
                local.fix_checked += 1;
                let ext = [
                    fixv(&data["startExtents"]["x"]),
                    fixv(&data["startExtents"]["y"]),
                ];
                fix_bad |= match &s.kind {
                    DetShapeKind::Box { extents, .. } => *extents != ext,
                    DetShapeKind::Circle { radius } => *radius != fixv(&data["startRadius"]),
                    DetShapeKind::RoundedRect { extents, radius } => {
                        *extents != ext || *radius != fixv(&data["startRadius"])
                    }
                };
            }
            if fix_bad {
                local.fix_bad += 1;
                if local.examples.len() < 5 {
                    local
                        .examples
                        .push(format!("{}: typed Fix values differ", c.id));
                }
            }
        }
    }
    let mut g = st.lock().unwrap();
    g.compared += local.compared;
    g.equal += local.equal;
    g.mismatches += local.mismatches;
    g.no_ref += local.no_ref;
    g.fix_checked += local.fix_checked;
    g.fix_bad += local.fix_bad;
    for e in local.examples {
        if g.examples.len() < 20 {
            g.examples.push(e);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var("BOPL_ROOT").unwrap_or_else(|_| ".".into()));
    let mut data = root.join("cache/import");
    let mut dump = root.join("ref/dump");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data = args.next().expect("--data DIR").into(),
            "--dump" => dump = args.next().expect("--dump DIR").into(),
            _ => anyhow::bail!("unknown argument {a}"),
        }
    }
    if !Path::new(&dump).is_dir() {
        anyhow::bail!("no reference dump at {}", dump.display());
    }
    let t = std::time::Instant::now();
    let gd = GameData::load(&data)?;
    let dumps = Dumps {
        dir: dump,
        cache: Default::default(),
    };
    let mut per_kind: BTreeMap<&str, Stats> = BTreeMap::new();
    let levels = Mutex::new(Stats::default());
    gd.level_indices().par_iter().try_for_each(|&i| {
        check_objects(&gd.level(i)?.objects, &dumps, &levels);
        anyhow::Ok(())
    })?;
    per_kind.insert("levels", levels.into_inner().unwrap());
    let scenes = Mutex::new(Stats::default());
    for e in gd
        .index
        .scenes
        .iter()
        .filter(|e| !e.level && e.file.is_some())
    {
        check_objects(&gd.scene(e.build_index)?.objects, &dumps, &scenes);
    }
    per_kind.insert("scenes", scenes.into_inner().unwrap());
    let prefabs = Mutex::new(Stats::default());
    let roots: Vec<&String> = gd.prefabs.roots.keys().collect();
    roots.par_iter().try_for_each(|id| {
        check_objects(&gd.prefab(id)?.objects, &dumps, &prefabs);
        anyhow::Ok(())
    })?;
    per_kind.insert("prefabs", prefabs.into_inner().unwrap());
    let mut failed = false;
    println!(
        "{:<8} {:>9} {:>8} {:>9} {:>6} {:>10} {:>8}",
        "kind", "compared", "equal", "mismatch", "no-ref", "fix-typed", "fix-bad"
    );
    for (k, s) in &per_kind {
        println!(
            "{k:<8} {:>9} {:>8} {:>9} {:>6} {:>10} {:>8}",
            s.compared, s.equal, s.mismatches, s.no_ref, s.fix_checked, s.fix_bad
        );
        for e in &s.examples {
            println!("    {e}");
        }
        failed |= s.mismatches > 0 || s.fix_bad > 0;
    }
    println!("{:.1}s", t.elapsed().as_secs_f64());
    if failed {
        std::process::exit(1);
    }
    Ok(())
}
