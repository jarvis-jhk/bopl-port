//! Dynamic values produced by the type-tree interpreter.

use std::sync::Arc;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

/// A reference to an object: `file_id` 0 is the same file, `n` is
/// `externals[n - 1]` of the referencing file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PPtr {
    /// File index (see above).
    pub file_id: i32,
    /// Path ID inside that file; 0 means null.
    pub path_id: i64,
}

impl PPtr {
    /// Whether the reference is null.
    pub fn is_null(&self) -> bool {
        self.path_id == 0
    }
}

/// A structured value (class or struct instance) with ordered fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    /// The node type, e.g. `Vector3f`, `PPtr<GameObject>`, `CardInfo`.
    pub type_name: Arc<str>,
    /// Fields in serialization order.
    pub fields: Vec<(Arc<str>, Value)>,
}

impl Object {
    /// Looks up a field by name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(n, _)| &**n == name)
            .map(|(_, v)| v)
    }
}

/// A value read through a type tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Absent value (e.g. a null managed reference).
    Null,
    /// `bool`.
    Bool(bool),
    /// Signed integers of any width.
    Int(i64),
    /// Unsigned integers of any width.
    UInt(u64),
    /// `float`.
    Float(f32),
    /// `double`.
    Double(f64),
    /// `string`.
    String(String),
    /// Arrays of `UInt8`/`char` and `TypelessData`.
    Bytes(Vec<u8>),
    /// Any other array.
    Array(Vec<Value>),
    /// Classes and structs.
    Object(Object),
}

static NULL: Value = Value::Null;

impl Value {
    /// Field lookup on objects; `Null` for anything else or missing fields.
    pub fn get(&self, name: &str) -> &Value {
        match self {
            Value::Object(o) => o.get(name).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    /// Nested lookup with a dotted path, e.g. `m_RD.texture`.
    pub fn path(&self, path: &str) -> &Value {
        path.split('.').fold(self, |v, k| v.get(k))
    }

    /// Whether the value is `Null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The object, if this is one.
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Integer view of any numeric or boolean value.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(v) => Some(v),
            Value::UInt(v) => i64::try_from(v).ok(),
            Value::Bool(b) => Some(b as i64),
            Value::Float(f) if f.fract() == 0.0 => Some(f as i64),
            _ => None,
        }
    }

    /// Unsigned integer view.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Value::UInt(v) => Some(v),
            Value::Int(v) => u64::try_from(v).ok(),
            Value::Bool(b) => Some(b as u64),
            _ => None,
        }
    }

    /// Float view of any numeric value.
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Value::Float(f) => Some(f),
            Value::Double(f) => Some(f as f32),
            Value::Int(v) => Some(v as f32),
            Value::UInt(v) => Some(v as f32),
            _ => None,
        }
    }

    /// Boolean view (`bool` or a non-zero integer, as Unity stores `UInt8` flags).
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            Value::Int(v) => Some(v != 0),
            Value::UInt(v) => Some(v != 0),
            _ => None,
        }
    }

    /// String view.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Byte view.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Array view (`Bytes` are not included).
    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }

    /// Float with a default of 0.
    pub fn f32(&self) -> f32 {
        self.as_f32().unwrap_or(0.0)
    }

    /// Integer with a default of 0.
    pub fn i64(&self) -> i64 {
        self.as_i64().unwrap_or(0)
    }

    /// Boolean with a default of `false`.
    pub fn bool(&self) -> bool {
        self.as_bool().unwrap_or(false)
    }

    /// String with a default of `""`.
    pub fn str(&self) -> &str {
        self.as_str().unwrap_or("")
    }

    /// Interprets an object with `m_FileID`/`m_PathID` as a [`PPtr`].
    pub fn as_pptr(&self) -> Option<PPtr> {
        let o = self.as_object()?;
        Some(PPtr {
            file_id: o.get("m_FileID")?.as_i64()? as i32,
            path_id: o.get("m_PathID")?.as_i64()?,
        })
    }

    /// Whether this is a PPtr object.
    pub fn is_pptr(&self) -> bool {
        self.as_object()
            .is_some_and(|o| o.type_name.starts_with("PPtr<"))
    }

    /// `[x, y]` of a `Vector2f`.
    pub fn vec2(&self) -> [f32; 2] {
        [self.get("x").f32(), self.get("y").f32()]
    }

    /// `[x, y, z]` of a `Vector3f`.
    pub fn vec3(&self) -> [f32; 3] {
        [
            self.get("x").f32(),
            self.get("y").f32(),
            self.get("z").f32(),
        ]
    }

    /// `[x, y, z, w]` of a `Vector4f`/`Quaternionf`.
    pub fn vec4(&self) -> [f32; 4] {
        [
            self.get("x").f32(),
            self.get("y").f32(),
            self.get("z").f32(),
            self.get("w").f32(),
        ]
    }

    /// `[r, g, b, a]` of a `ColorRGBA`.
    pub fn color(&self) -> [f32; 4] {
        [
            self.get("r").f32(),
            self.get("g").f32(),
            self.get("b").f32(),
            self.get("a").f32(),
        ]
    }

    /// `[x, y, width, height]` of a `Rectf`.
    pub fn rect(&self) -> [f32; 4] {
        [
            self.get("x").f32(),
            self.get("y").f32(),
            self.get("width").f32(),
            self.get("height").f32(),
        ]
    }

    /// Calls `f` on every PPtr inside this value (depth first).
    pub fn visit_pptrs(&self, f: &mut impl FnMut(PPtr)) {
        match self {
            Value::Object(o) => {
                if o.type_name.starts_with("PPtr<") {
                    if let Some(p) = self.as_pptr() {
                        f(p);
                    }
                    return;
                }
                for (_, v) in &o.fields {
                    v.visit_pptrs(f);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| v.visit_pptrs(f)),
            _ => {}
        }
    }
}

/// JSON-like serialization: objects become maps, PPtrs `{m_FileID, m_PathID}`,
/// bytes an array of numbers, non-finite floats `null` (via serde_json).
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(v) => s.serialize_i64(*v),
            Value::UInt(v) => s.serialize_u64(*v),
            Value::Float(v) => s.serialize_f32(*v),
            Value::Double(v) => s.serialize_f64(*v),
            Value::String(v) => s.serialize_str(v),
            Value::Bytes(b) => {
                let mut seq = s.serialize_seq(Some(b.len()))?;
                for x in b {
                    seq.serialize_element(x)?;
                }
                seq.end()
            }
            Value::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for x in a {
                    seq.serialize_element(x)?;
                }
                seq.end()
            }
            Value::Object(o) => {
                let mut m = s.serialize_map(Some(o.fields.len()))?;
                for (k, v) in &o.fields {
                    m.serialize_entry(&**k, v)?;
                }
                m.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(t: &str, f: Vec<(&str, Value)>) -> Value {
        Value::Object(Object {
            type_name: t.into(),
            fields: f.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        })
    }

    #[test]
    fn accessors() {
        let p = obj(
            "PPtr<Sprite>",
            vec![("m_FileID", Value::Int(2)), ("m_PathID", Value::Int(9))],
        );
        let v = obj(
            "X",
            vec![("a", obj("Y", vec![("b", Value::Float(1.5))])), ("p", p)],
        );
        assert_eq!(v.path("a.b").f32(), 1.5);
        assert!(v.path("a.c").is_null());
        assert_eq!(
            v.get("p").as_pptr(),
            Some(PPtr {
                file_id: 2,
                path_id: 9
            })
        );
        let mut seen = vec![];
        v.visit_pptrs(&mut |p| seen.push(p.path_id));
        assert_eq!(seen, [9]);
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"a":{"b":1.5},"p":{"m_FileID":2,"m_PathID":9}}"#
        );
    }
}
