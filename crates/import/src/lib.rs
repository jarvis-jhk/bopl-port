//! `bopl-import`: converts Bopl Battle's Unity data into bopl-port's own
//! versioned format (PNG textures, Ogg audio, JSON for sprites, levels,
//! scenes, prefabs, abilities, global lists, fonts, localisation), and loads
//! it back ([`GameData`]). The format is documented in
//! `docs/formats/import.md`.

pub mod anim;
pub mod fonts;
pub mod format;
pub mod import;
pub mod load;
pub mod particles;
pub mod paths;
pub mod scene;

use std::collections::BTreeSet;

pub use format::{FORMAT_REVISION, FORMAT_VERSION};
pub use import::{Options, Report, import, is_up_to_date};
pub use load::GameData;

/// Directory name of an import of the current format inside the user data dir.
pub fn versioned_dir_name() -> String {
    format!("import-v{FORMAT_VERSION}")
}

/// Collects all `{"$ref": id}` targets in a raw JSON field tree.
pub fn collect_refs(j: &serde_json::Value, out: &mut BTreeSet<format::ObjId>) {
    use serde_json::Value as J;
    match j {
        J::Object(m) => {
            if let Some(J::String(id)) = m.get("$ref") {
                out.insert(id.clone());
                return;
            }
            m.values().for_each(|v| collect_refs(v, out));
        }
        J::Array(a) => a.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_collected() {
        let j = serde_json::json!({"a": {"$ref": "x:1"}, "b": [{"$ref": "y:2"}, null, {"c": {"$ref": "x:1"}}]});
        let mut s = BTreeSet::new();
        collect_refs(&j, &mut s);
        assert_eq!(s.into_iter().collect::<Vec<_>>(), ["x:1", "y:2"]);
    }

    #[test]
    fn dir_name() {
        assert_eq!(versioned_dir_name(), format!("import-v{FORMAT_VERSION}"));
    }
}
