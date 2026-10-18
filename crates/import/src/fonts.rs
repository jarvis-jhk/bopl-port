//! TextMeshPro font assets (`fonts.json`): glyph-table repair and the typed
//! [`format::FontInfo`].
//!
//! Font assets created with TMP 1.x stored glyphs in the legacy
//! `m_glyphInfoList` (`x`, `y` from the atlas's top-left corner). When TMP
//! 1.4+ upgrades such an asset it converts each entry to a `m_GlyphTable`
//! rect with a bottom-left origin, `y' = atlasHeight - (y + height)`.
//! `LiberationSans SDF` (TMP's default font, shipped upgraded) had
//! placeholder positions for its whitespace glyphs (space, no-break space,
//! U+2000-U+200B, U+202F) below the atlas (`y = 1033` in a 1024 atlas), so
//! the converted rects are negative (`y' = -105`, or `-9` for the empty
//! ones). TMP never draws whitespace, so the game is unaffected, but a
//! renderer that validates rects would drop those glyphs and lose their
//! advances. We replace every rect that lies outside the atlas by an empty
//! rect at 0,0 (and zero width/height), keeping bearings and advances.

use std::collections::BTreeMap;

use serde_json::Value as J;

use crate::format;

fn num(j: &J, k: &str) -> f32 {
    j.get(k).and_then(J::as_f64).unwrap_or(0.0) as f32
}

fn int(j: &J, k: &str) -> i64 {
    j.get(k)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

/// Whether a glyph rect `[x, y, w, h]` lies inside a `w`x`h` atlas.
fn rect_ok(r: [i64; 4], aw: i64, ah: i64) -> bool {
    r[0] >= 0 && r[1] >= 0 && r[2] >= 0 && r[3] >= 0 && r[0] + r[2] <= aw && r[1] + r[3] <= ah
}

/// Repairs out-of-atlas glyph rects in the raw fields of a `TMP_FontAsset`
/// and builds its [`format::FontInfo`]. Returns the info and the code points
/// (or `U+?` glyph indices without a character) of the repaired glyphs that
/// are not whitespace (which would indicate real damage).
pub fn font_info(fields: &mut J) -> (format::FontInfo, Vec<String>) {
    let aw = int(fields, "m_AtlasWidth");
    let ah = int(fields, "m_AtlasHeight");
    let characters: BTreeMap<u32, u32> = fields
        .get("m_CharacterTable")
        .and_then(J::as_array)
        .map(|a| {
            a.iter()
                .map(|c| (int(c, "m_Unicode") as u32, int(c, "m_GlyphIndex") as u32))
                .collect()
        })
        .unwrap_or_default();
    let mut suspicious = Vec::new();
    let mut glyphs = Vec::new();
    if let Some(table) = fields.get_mut("m_GlyphTable").and_then(J::as_array_mut) {
        for g in table.iter_mut() {
            let index = int(g, "m_Index") as u32;
            let r = g.get("m_GlyphRect").cloned().unwrap_or(J::Null);
            let mut rect = [
                int(&r, "m_X"),
                int(&r, "m_Y"),
                int(&r, "m_Width"),
                int(&r, "m_Height"),
            ];
            let fixed = !rect_ok(rect, aw, ah);
            if fixed {
                rect = [0; 4];
                let chars: Vec<u32> = characters
                    .iter()
                    .filter(|(_, gi)| **gi == index)
                    .map(|(u, _)| *u)
                    .collect();
                let ws =
                    |u: &u32| char::from_u32(*u).is_some_and(|c| c.is_whitespace()) || *u == 0x200B;
                if chars.is_empty() {
                    suspicious.push(format!("glyph {index}"));
                } else if !chars.iter().all(ws) {
                    suspicious.extend(chars.iter().map(|u| format!("U+{u:04X}")));
                }
                g["m_GlyphRect"] = serde_json::json!({
                    "m_X": 0, "m_Y": 0, "m_Width": 0, "m_Height": 0
                });
                if let Some(m) = g.get_mut("m_Metrics").and_then(J::as_object_mut) {
                    m.insert("m_Width".into(), J::from(0.0));
                    m.insert("m_Height".into(), J::from(0.0));
                }
            }
            let m = g.get("m_Metrics").cloned().unwrap_or(J::Null);
            let [x, y, w, h] = rect.map(|v| v as f32);
            glyphs.push(format::FontGlyph {
                index,
                atlas: int(g, "m_AtlasIndex").max(0) as u32,
                rect: [x, y, w, h],
                rect_png: [x, ah as f32 - y - h, w, h],
                width: num(&m, "m_Width"),
                height: num(&m, "m_Height"),
                bearing_x: num(&m, "m_HorizontalBearingX"),
                bearing_y: num(&m, "m_HorizontalBearingY"),
                advance: num(&m, "m_HorizontalAdvance"),
                scale: g.get("m_Scale").and_then(J::as_f64).unwrap_or(1.0) as f32,
            });
        }
    }
    let value = |r: &J| -> [f32; 4] {
        let v = r.get("m_GlyphValueRecord").cloned().unwrap_or(J::Null);
        [
            num(&v, "m_XPlacement"),
            num(&v, "m_YPlacement"),
            num(&v, "m_XAdvance"),
            num(&v, "m_YAdvance"),
        ]
    };
    let kerning = fields
        .pointer("/m_FontFeatureTable/m_GlyphPairAdjustmentRecords")
        .and_then(J::as_array)
        .map(|a| {
            a.iter()
                .map(|k| {
                    let f = k.get("m_FirstAdjustmentRecord").cloned().unwrap_or(J::Null);
                    let s = k
                        .get("m_SecondAdjustmentRecord")
                        .cloned()
                        .unwrap_or(J::Null);
                    format::FontKerning {
                        first: int(&f, "m_GlyphIndex") as u32,
                        second: int(&s, "m_GlyphIndex") as u32,
                        first_adjust: value(&f),
                        second_adjust: value(&s),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let face = fields.get("m_FaceInfo").cloned().unwrap_or(J::Null);
    let info = format::FontInfo {
        atlas_width: aw.max(0) as u32,
        atlas_height: ah.max(0) as u32,
        padding: num(fields, "m_AtlasPadding"),
        render_mode: int(fields, "m_AtlasRenderMode"),
        point_size: num(&face, "m_PointSize"),
        scale: num(&face, "m_Scale"),
        line_height: num(&face, "m_LineHeight"),
        ascent: num(&face, "m_AscentLine"),
        descent: num(&face, "m_DescentLine"),
        baseline: num(&face, "m_Baseline"),
        cap_line: num(&face, "m_CapLine"),
        mean_line: num(&face, "m_MeanLine"),
        underline_offset: num(&face, "m_UnderlineOffset"),
        underline_thickness: num(&face, "m_UnderlineThickness"),
        tab_width: num(&face, "m_TabWidth"),
        bold_style: num(fields, "boldStyle"),
        bold_spacing: num(fields, "boldSpacing"),
        glyphs,
        characters,
        kerning,
    };
    (info, suspicious)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn negative_whitespace_rects_are_emptied() {
        let mut f = json!({
            "m_AtlasWidth": 1024, "m_AtlasHeight": 1024, "m_AtlasPadding": 9,
            "m_FaceInfo": {"m_PointSize": 86, "m_LineHeight": 98.9, "m_AscentLine": 77.8},
            "m_CharacterTable": [
                {"m_Unicode": 32, "m_GlyphIndex": 1},
                {"m_Unicode": 65, "m_GlyphIndex": 2}
            ],
            "m_GlyphTable": [
                {"m_Index": 1, "m_AtlasIndex": 0, "m_Scale": 1.0,
                 "m_GlyphRect": {"m_X": 10, "m_Y": -105, "m_Width": 24, "m_Height": 96},
                 "m_Metrics": {"m_Width": 23.9, "m_Height": 96.0, "m_HorizontalBearingX": 0.0,
                               "m_HorizontalBearingY": 77.8, "m_HorizontalAdvance": 23.9}},
                {"m_Index": 2, "m_AtlasIndex": 0, "m_Scale": 1.0,
                 "m_GlyphRect": {"m_X": 100, "m_Y": 200, "m_Width": 50, "m_Height": 60},
                 "m_Metrics": {"m_Width": 50.0, "m_Height": 60.0, "m_HorizontalBearingX": 1.0,
                               "m_HorizontalBearingY": 59.0, "m_HorizontalAdvance": 55.0}}
            ],
            "m_FontFeatureTable": {"m_GlyphPairAdjustmentRecords": [
                {"m_FirstAdjustmentRecord": {"m_GlyphIndex": 1, "m_GlyphValueRecord": {"m_XAdvance": -4.5}},
                 "m_SecondAdjustmentRecord": {"m_GlyphIndex": 2, "m_GlyphValueRecord": {}}}
            ]}
        });
        let (info, bad) = font_info(&mut f);
        assert!(bad.is_empty());
        assert_eq!(f["m_GlyphTable"][0]["m_GlyphRect"]["m_Y"], 0);
        assert_eq!(f["m_GlyphTable"][1]["m_GlyphRect"]["m_Y"], 200);
        let space = info.glyph_for(' ').unwrap();
        assert_eq!(space.rect, [0.0; 4]);
        assert_eq!(space.advance, 23.9);
        let a = info.glyph_for('A').unwrap();
        assert_eq!(a.rect, [100.0, 200.0, 50.0, 60.0]);
        assert_eq!(a.rect_png, [100.0, 1024.0 - 260.0, 50.0, 60.0]);
        assert_eq!(info.kerning_x(1, 2), -4.5);
        assert_eq!(info.padding, 9.0);
        assert_eq!(info.point_size, 86.0);
    }

    #[test]
    fn non_whitespace_repairs_are_reported() {
        let mut f = json!({
            "m_AtlasWidth": 64, "m_AtlasHeight": 64,
            "m_CharacterTable": [{"m_Unicode": 66, "m_GlyphIndex": 3}],
            "m_GlyphTable": [{"m_Index": 3, "m_GlyphRect": {"m_X": 60, "m_Y": 0, "m_Width": 10, "m_Height": 10}}]
        });
        let (_, bad) = font_info(&mut f);
        assert_eq!(bad, ["U+0042"]);
    }
}
