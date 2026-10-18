//! Dev tool: reads every object of every SerializedFile of the game with
//! `unity-assets` and compares the result against the UnityPy reference dump
//! (`ref/dump/<file>.json`, from `scripts/dev/dump_assets.py`).
//!
//! Usage: `cargo run --release -p unity-assets --example validate -- [--game DIR] [--dump DIR] [FILE...]`
//!
//! Per class it reports: objects, layouts that consumed exactly the stored
//! bytes, values equal to the dump, mismatches, objects the dump could not
//! read, read errors. MonoBehaviour `m_Script` is not compared (the dump's
//! generated header is misaligned there, see the dump script).

use std::collections::BTreeMap;
use std::path::PathBuf;

use rayon::prelude::*;
use serde_json::Value as J;
use unity_assets::{AssetDir, Value};

#[derive(Default, Clone)]
struct Stats {
    objects: usize,
    exact: usize,
    equal: usize,
    mismatch: usize,
    no_ref: usize,
    errors: usize,
    examples: Vec<String>,
}

impl Stats {
    fn add(&mut self, o: &Stats) {
        self.objects += o.objects;
        self.exact += o.exact;
        self.equal += o.equal;
        self.mismatch += o.mismatch;
        self.no_ref += o.no_ref;
        self.errors += o.errors;
        for e in &o.examples {
            if self.examples.len() < max_examples() {
                self.examples.push(e.clone());
            }
        }
    }
}

/// Python's json writes bare `NaN`/`Infinity`; quote them so serde_json parses.
fn quote_non_finite(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut in_str, mut esc) = (0, false, false);
    let mut last = 0;
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
        let mut matched = false;
        for tok in ["-Infinity", "Infinity", "NaN"] {
            if b[i..].starts_with(tok.as_bytes()) {
                matched = true;
                out.push_str(&s[last..i]);
                out.push('"');
                out.push_str(tok);
                out.push('"');
                i += tok.len();
                last = i;
                break;
            }
        }
        if !matched {
            i += 1;
        }
    }
    out.push_str(&s[last.min(s.len())..]);
    out
}

fn max_examples() -> usize {
    std::env::var("VALIDATE_EXAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

fn float_eq(mine: f64, d: &J, single: bool) -> bool {
    let theirs = match d {
        J::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        J::String(s) if s == "NaN" => return mine.is_nan(),
        J::String(s) if s == "Infinity" => return mine == f64::INFINITY,
        J::String(s) if s == "-Infinity" => return mine == f64::NEG_INFINITY,
        _ => return false,
    };
    if single {
        mine as f32 == theirs as f32
    } else {
        mine == theirs
    }
}

fn cmp(m: &Value, d: &J, path: &str, out: &mut Option<String>) {
    if out.is_some() {
        return;
    }
    let bad = |out: &mut Option<String>, why: &str| {
        let ds = d.to_string();
        *out = Some(format!("{path}: {why} (dump {})", &ds[..ds.len().min(80)]));
    };
    if let J::Object(o) = d {
        if let Some(n) = o.get("$bytes").and_then(|n| n.as_u64()) {
            let len = match m {
                Value::Bytes(b) => b.len(),
                Value::Array(a) => a.len(),
                _ => usize::MAX,
            };
            if len as u64 != n {
                bad(out, &format!("byte length {len}"));
            }
            return;
        }
        if let Some(n) = o.get("$array").and_then(|n| n.as_u64()) {
            let len = match m {
                Value::Bytes(b) => b.len(),
                _ => m.as_array().len(),
            };
            if len as u64 != n {
                bad(out, "array length");
            }
            return;
        }
    }
    match (m, d) {
        (Value::Null, J::Null) => {}
        (Value::Bool(a), J::Bool(b)) if a == b => {}
        (Value::Int(a), J::Number(n)) if n.as_i64() == Some(*a) => {}
        (Value::UInt(a), J::Number(n)) if n.as_u64() == Some(*a) => {}
        (Value::Float(a), _) if float_eq(*a as f64, d, true) => {}
        (Value::Double(a), _) if float_eq(*a, d, false) => {}
        (Value::String(a), J::String(b)) if a == b => {}
        (Value::String(_), J::String(_)) => bad(out, &format!("string {m:?}")),
        (Value::Bytes(a), J::Array(b)) => {
            if a.len() != b.len() || a.iter().zip(b).any(|(x, y)| y.as_u64() != Some(*x as u64)) {
                bad(out, "bytes");
            }
        }
        (Value::Array(a), J::Array(b)) => {
            if a.len() != b.len() {
                return bad(out, &format!("array length {} vs {}", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                cmp(x, y, &format!("{path}[{i}]"), out);
            }
        }
        (Value::Object(o), J::Array(b)) if &*o.type_name == "pair" && b.len() == 2 => {
            cmp(
                o.get("first").unwrap_or(&Value::Null),
                &b[0],
                &format!("{path}.first"),
                out,
            );
            cmp(
                o.get("second").unwrap_or(&Value::Null),
                &b[1],
                &format!("{path}.second"),
                out,
            );
        }
        (Value::Object(o), J::Object(b)) => {
            let mine: Vec<&str> = o.fields.iter().map(|(k, _)| &**k).collect();
            let theirs: Vec<&str> = b.keys().map(|k| k.as_str()).collect();
            if mine.len() != theirs.len() || mine.iter().any(|k| !b.contains_key(*k)) {
                return bad(out, &format!("fields {mine:?} vs {theirs:?}"));
            }
            for (k, v) in &o.fields {
                if path.is_empty() && &**k == "m_Script" {
                    continue;
                }
                cmp(v, &b[&**k], &format!("{path}.{k}"), out);
            }
        }
        _ => bad(out, &format!("value {:?}", short(m))),
    }
}

fn short(v: &Value) -> String {
    let s = format!("{v:?}");
    s[..s.len().min(60)].to_owned()
}

fn validate_file(
    dir: &AssetDir,
    dump_dir: &std::path::Path,
    name: &str,
) -> BTreeMap<String, Stats> {
    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    let file = match dir.file(name) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{name}: {e}");
            return stats;
        }
    };
    let dump: Option<J> = std::fs::read_to_string(dump_dir.join(format!("{name}.json")))
        .ok()
        .and_then(|s| {
            serde_json::from_str(&quote_non_finite(&s))
                .map_err(|e| eprintln!("{name}: dump: {e}"))
                .ok()
        });
    let objs = dump.as_ref().map(|d| &d["objects"]);
    for info in &file.meta.objects {
        let class = dir
            .registry()
            .class_name(info.class_id)
            .unwrap_or("?")
            .to_owned();
        let st = stats
            .entry(format!("{class} ({})", info.class_id))
            .or_default();
        st.objects += 1;
        let r = match dir.read_object(&file, info) {
            Ok(r) => r,
            Err(e) => {
                st.errors += 1;
                if st.examples.len() < max_examples() {
                    st.examples
                        .push(format!("{name}:{} read error: {e}", info.path_id));
                }
                continue;
            }
        };
        if r.exact() {
            st.exact += 1;
        } else if st.examples.len() < max_examples() {
            st.examples.push(format!(
                "{name}:{} {:?}: read {} of {} bytes{}",
                info.path_id,
                r.script.as_ref().map(|s| s.full_name()),
                r.consumed,
                r.size,
                if r.full_layout { "" } else { " (no layout)" }
            ));
        }
        let Some(d) = objs
            .and_then(|o| o.get(info.path_id.to_string()))
            .and_then(|o| o.get("data"))
        else {
            st.no_ref += 1;
            continue;
        };
        if !r.full_layout {
            st.no_ref += 1;
            continue;
        }
        let mut diff = None;
        cmp(&r.value, d, "", &mut diff);
        match diff {
            None => st.equal += 1,
            Some(msg) => {
                st.mismatch += 1;
                if st.examples.len() < max_examples() {
                    st.examples.push(format!("{name}:{} {msg}", info.path_id));
                }
            }
        }
    }
    stats
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::env::var("BOPL_ROOT").unwrap_or_else(|_| ".".into());
    let mut game = std::env::var("BOPL_GAME_DIR").unwrap_or_default();
    let mut dump = format!("{root}/ref/dump");
    let mut only = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game" => game = args.next().expect("--game DIR"),
            "--dump" => dump = args.next().expect("--dump DIR"),
            _ => only.push(a),
        }
    }
    let dir = AssetDir::open(&game).expect("open game dir");
    let names = if only.is_empty() {
        dir.serialized_file_names().unwrap()
    } else {
        only
    };
    let dump = PathBuf::from(dump);
    let t = std::time::Instant::now();
    let per_file: Vec<_> = names
        .par_iter()
        .map(|n| validate_file(&dir, &dump, n))
        .collect();
    let mut total: BTreeMap<String, Stats> = BTreeMap::new();
    for m in per_file {
        for (k, v) in m {
            total.entry(k).or_default().add(&v);
        }
    }
    println!(
        "{:<38} {:>7} {:>7} {:>7} {:>8} {:>6} {:>6}",
        "class", "objects", "exact", "equal", "mismatch", "no-ref", "errors"
    );
    let mut sum = Stats::default();
    for (k, s) in &total {
        println!(
            "{k:<38} {:>7} {:>7} {:>7} {:>8} {:>6} {:>6}",
            s.objects, s.exact, s.equal, s.mismatch, s.no_ref, s.errors
        );
        for e in &s.examples {
            println!("    {e}");
        }
        sum.add(&Stats {
            examples: vec![],
            ..s.clone()
        });
    }
    println!(
        "{:<38} {:>7} {:>7} {:>7} {:>8} {:>6} {:>6}",
        "TOTAL", sum.objects, sum.exact, sum.equal, sum.mismatch, sum.no_ref, sum.errors
    );
    println!("{} files in {:.1}s", names.len(), t.elapsed().as_secs_f64());
}
