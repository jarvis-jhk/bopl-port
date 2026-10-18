//! Type trees and the generic interpreter that reads object data with them.
//!
//! A type tree is Unity's description of a serialized layout: a pre-order list
//! of nodes `(level, type, name, byte size, meta flags)`. Reading rules (Unity
//! 2022.3 binary serialization):
//!
//! * primitives (`SInt8` .. `double`, `bool`) are stored inline, unaligned;
//! * `string` is an `i32` byte length, UTF-8 bytes, then 4-byte alignment;
//! * a node whose first child is `Array` is an array: `i32` count, then the
//!   elements described by the `Array` node's second child (`data`);
//! * `TypelessData` is an `i32` count followed by raw bytes;
//! * anything else is a struct: its children in order;
//! * after any node with meta flag `0x4000` (or whose `Array` child has it),
//!   the stream is aligned to 4 bytes.

use std::sync::Arc;

use crate::bytes::Reader;
use crate::value::{Object, Value};
use crate::{Error, Result};

/// Meta flag: align the stream to 4 bytes after this node.
pub const ALIGN_BYTES: u32 = 0x4000;

/// A node as stored in type-tree data, before compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawNode {
    /// Depth in the tree (root = 0).
    pub level: u8,
    /// Type name, e.g. `int`, `Vector3f`, `PPtr<Transform>`.
    pub type_name: String,
    /// Field name.
    pub name: String,
    /// Fixed byte size or -1 (informational).
    pub byte_size: i32,
    /// Meta flags (`ALIGN_BYTES`, ...).
    pub meta: u32,
}

impl RawNode {
    /// Convenience constructor.
    pub fn new(level: u8, type_name: &str, name: &str, byte_size: i32, meta: u32) -> Self {
        RawNode {
            level,
            type_name: type_name.into(),
            name: name.into(),
            byte_size,
            meta,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prim {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Prim {
    fn parse(t: &str) -> Option<Prim> {
        Some(match t {
            "bool" => Prim::Bool,
            "SInt8" => Prim::I8,
            "UInt8" | "char" => Prim::U8,
            "short" | "SInt16" => Prim::I16,
            "unsigned short" | "UInt16" => Prim::U16,
            "int" | "SInt32" => Prim::I32,
            "unsigned int" | "UInt32" | "Type*" => Prim::U32,
            "long long" | "SInt64" => Prim::I64,
            "unsigned long long" | "UInt64" | "FileSize" => Prim::U64,
            "float" => Prim::F32,
            "double" => Prim::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Prim::Bool | Prim::I8 | Prim::U8 => 1,
            Prim::I16 | Prim::U16 => 2,
            Prim::I32 | Prim::U32 | Prim::F32 => 4,
            Prim::I64 | Prim::U64 | Prim::F64 => 8,
        }
    }

    #[inline]
    fn read(self, r: &mut Reader) -> Result<Value> {
        Ok(match self {
            Prim::Bool => Value::Bool(r.u8()? != 0),
            Prim::I8 => Value::Int(r.u8()? as i8 as i64),
            Prim::U8 => Value::UInt(r.u8()? as u64),
            Prim::I16 => Value::Int(r.i16()? as i64),
            Prim::U16 => Value::UInt(r.u16()? as u64),
            Prim::I32 => Value::Int(r.i32()? as i64),
            Prim::U32 => Value::UInt(r.u32()? as u64),
            Prim::I64 => Value::Int(r.i64()?),
            Prim::U64 => Value::UInt(r.u64()?),
            Prim::F32 => Value::Float(r.f32()?),
            Prim::F64 => Value::Double(r.f64()?),
        })
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Prim(Prim),
    String,
    TypelessData,
    /// Array with the index of its element node.
    Array(usize),
    /// `ReferencedObjectData`: payload typed by the sibling `type` field.
    ManagedData,
    Struct,
}

#[derive(Clone, Debug)]
struct Node {
    type_name: Arc<str>,
    name: Arc<str>,
    kind: Kind,
    align: bool,
    children: Vec<usize>,
}

/// A compiled type tree, ready for reading.
#[derive(Clone, Debug)]
pub struct TypeTree {
    nodes: Vec<Node>,
    raw: Vec<RawNode>,
}

impl TypeTree {
    /// Compiles a pre-order node list (the first node is the root).
    pub fn new(raw: Vec<RawNode>) -> Result<Self> {
        if raw.is_empty() {
            return Err(Error::Format("empty type tree".into()));
        }
        let mut nodes: Vec<Node> = Vec::with_capacity(raw.len());
        let mut stack: Vec<usize> = Vec::new();
        let mut names: std::collections::HashMap<String, Arc<str>> = Default::default();
        let mut intern = |s: &str| -> Arc<str> {
            names
                .entry(s.to_owned())
                .or_insert_with(|| s.into())
                .clone()
        };
        for (i, r) in raw.iter().enumerate() {
            while let Some(&top) = stack.last() {
                if raw[top].level >= r.level {
                    stack.pop();
                } else {
                    break;
                }
            }
            match stack.last() {
                Some(&parent) => {
                    if raw[parent].level + 1 != r.level {
                        return Err(Error::Format(format!("node {i} skips a level")));
                    }
                    nodes[parent].children.push(i);
                }
                None if i != 0 => return Err(Error::Format(format!("node {i} has no parent"))),
                None => {}
            }
            nodes.push(Node {
                type_name: intern(&r.type_name),
                name: intern(&r.name),
                kind: Kind::Struct,
                align: r.meta & ALIGN_BYTES != 0,
                children: Vec::new(),
            });
            stack.push(i);
        }
        for i in 0..nodes.len() {
            let t = &*nodes[i].type_name;
            let kind = if let Some(p) = Prim::parse(t) {
                Kind::Prim(p)
            } else if t == "string" && is_plain_string(&nodes, i) {
                Kind::String
            } else if t == "TypelessData" {
                Kind::TypelessData
            } else if t == "ReferencedObjectData" {
                Kind::ManagedData
            } else if let Some(&first) = nodes[i].children.first()
                && &*nodes[first].type_name == "Array"
            {
                let arr = &nodes[first];
                let elem = *arr
                    .children
                    .get(1)
                    .ok_or_else(|| Error::Format(format!("array node {i} without element")))?;
                if arr.align {
                    nodes[i].align = true;
                }
                Kind::Array(elem)
            } else if t == "Array" {
                let elem = *nodes[i]
                    .children
                    .get(1)
                    .ok_or_else(|| Error::Format(format!("array node {i} without element")))?;
                Kind::Array(elem)
            } else {
                Kind::Struct
            };
            nodes[i].kind = kind;
        }
        Ok(TypeTree { nodes, raw })
    }

    /// The raw nodes this tree was compiled from.
    pub fn raw(&self) -> &[RawNode] {
        &self.raw
    }

    /// Type name of the root node (the class name).
    pub fn root_type(&self) -> &str {
        &self.nodes[0].type_name
    }

    /// Reads one value of the root type.
    pub fn read(&self, r: &mut Reader) -> Result<Value> {
        self.read_node(0, r, None)
    }

    /// Reads one value, resolving `[SerializeReference]` payload types.
    pub fn read_with(&self, r: &mut Reader, refs: &dyn RefTypes) -> Result<Value> {
        self.read_node(0, r, Some(refs))
    }

    /// Reads only the root's first `n` fields (e.g. a MonoBehaviour header).
    pub fn read_prefix(&self, r: &mut Reader, n: usize) -> Result<Value> {
        let root = &self.nodes[0];
        let mut fields = Vec::with_capacity(n);
        for &c in root.children.iter().take(n) {
            fields.push((self.nodes[c].name.clone(), self.read_node(c, r, None)?));
        }
        Ok(Value::Object(Object {
            type_name: root.type_name.clone(),
            fields,
        }))
    }

    fn read_node(&self, i: usize, r: &mut Reader, refs: Option<&dyn RefTypes>) -> Result<Value> {
        let node = &self.nodes[i];
        let v = match node.kind {
            Kind::Prim(p) => p.read(r)?,
            Kind::String => {
                let n = r.len_prefix(1)?;
                Value::String(String::from_utf8_lossy(r.take(n)?).into_owned())
                // Alignment after strings is unconditional in Unity.
            }
            Kind::TypelessData => {
                let n = r.len_prefix(1)?;
                Value::Bytes(r.take(n)?.to_vec())
            }
            Kind::Array(elem) => self.read_array(elem, r, refs)?,
            Kind::ManagedData => {
                return Err(Error::Unsupported("managed reference payloads".into()));
            }
            Kind::Struct => {
                let mut fields = Vec::with_capacity(node.children.len());
                for &c in &node.children {
                    let v = if matches!(self.nodes[c].kind, Kind::ManagedData) {
                        let ty = fields
                            .iter()
                            .find(|(n, _): &&(Arc<str>, Value)| &**n == "type")
                            .map(|(_, t)| t)
                            .unwrap_or(&Value::Null);
                        let class = ty.get("class").str();
                        if class.is_empty() {
                            // A null reference has no payload.
                            Value::Null
                        } else {
                            let (ns, asm) = (ty.get("ns").str(), ty.get("asm").str());
                            let tree =
                                refs.and_then(|f| f.ref_type(asm, ns, class))
                                    .ok_or_else(|| {
                                        Error::Unsupported(format!(
                                            "managed reference payload of type {asm}:{ns}.{class}"
                                        ))
                                    })?;
                            tree.read_node(0, r, refs)?
                        }
                    } else {
                        self.read_node(c, r, refs)?
                    };
                    fields.push((self.nodes[c].name.clone(), v));
                }
                Value::Object(Object {
                    type_name: node.type_name.clone(),
                    fields,
                })
            }
        };
        if node.align || matches!(node.kind, Kind::String) {
            r.align4();
        }
        Ok(v)
    }

    fn read_array(
        &self,
        elem: usize,
        r: &mut Reader,
        refs: Option<&dyn RefTypes>,
    ) -> Result<Value> {
        let e = &self.nodes[elem];
        if let Kind::Prim(p) = e.kind {
            let n = r.len_prefix(p.size())?;
            if p == Prim::U8 {
                return Ok(Value::Bytes(r.take(n)?.to_vec()));
            }
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(p.read(r)?);
            }
            if e.align {
                r.align4();
            }
            return Ok(Value::Array(out));
        }
        let n = r.len_prefix(1)?;
        let mut out = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            out.push(self.read_node(elem, r, refs)?);
        }
        Ok(Value::Array(out))
    }
}

/// Looks up the layouts of `[SerializeReference]` payload classes.
pub trait RefTypes {
    /// Tree for a plain serializable class (root + fields, no object header).
    fn ref_type(&self, assembly: &str, namespace: &str, class: &str) -> Option<&TypeTree>;
}

/// A `string` node is a real string unless its `Array` holds non-`char`
/// elements: TypeTreeGenerator labels `string[]` fields as `string`.
fn is_plain_string(nodes: &[Node], i: usize) -> bool {
    let Some(&arr) = nodes[i].children.first() else {
        return true;
    };
    match nodes[arr].children.get(1) {
        Some(&elem) => &*nodes[elem].type_name == "char",
        None => true,
    }
}

/// Parses an engine/script node list in the committed JSON format:
/// `[[level, type, name, byte_size, meta], ...]` (byte size optional).
pub fn raw_nodes_from_json(v: &serde_json::Value) -> Result<Vec<RawNode>> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::Format("nodes: not an array".into()))?;
    arr.iter()
        .map(|n| {
            let a = n
                .as_array()
                .ok_or_else(|| Error::Format("node: not an array".into()))?;
            let s = |i: usize| a.get(i).and_then(|x| x.as_str()).unwrap_or("");
            let num = |i: usize| a.get(i).and_then(|x| x.as_i64()).unwrap_or(0);
            let (size, meta) = if a.len() >= 5 {
                (num(3), num(4))
            } else {
                (-1, num(3))
            };
            Ok(RawNode {
                level: num(0) as u8,
                type_name: s(1).into(),
                name: s(2).into(),
                byte_size: size as i32,
                meta: meta as u32,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(nodes: &[(u8, &str, &str, u32)]) -> TypeTree {
        TypeTree::new(
            nodes
                .iter()
                .map(|&(l, t, n, m)| RawNode::new(l, t, n, -1, m))
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn struct_string_array_alignment() {
        let t = tree(&[
            (0, "Thing", "Base", 0),
            (1, "UInt8", "flag", ALIGN_BYTES),
            (1, "string", "name", 0),
            (2, "Array", "Array", ALIGN_BYTES),
            (3, "int", "size", 0),
            (3, "char", "data", 0),
            (1, "vector", "pts", 0),
            (2, "Array", "Array", 0),
            (3, "int", "size", 0),
            (3, "Vector2f", "data", 0),
            (4, "float", "x", 0),
            (4, "float", "y", 0),
            (1, "bool", "last", 0),
        ]);
        let mut d = vec![1u8, 0, 0, 0];
        d.extend_from_slice(&3i32.to_le_bytes());
        d.extend_from_slice(b"abc\0");
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&1.0f32.to_le_bytes());
        d.extend_from_slice(&2.0f32.to_le_bytes());
        d.push(1);
        let mut r = Reader::new(&d);
        let v = t.read(&mut r).unwrap();
        assert_eq!(r.pos(), d.len());
        assert_eq!(v.get("flag"), &Value::UInt(1));
        assert_eq!(v.get("name").str(), "abc");
        assert_eq!(v.get("pts").as_array()[0].vec2(), [1.0, 2.0]);
        assert_eq!(v.get("last"), &Value::Bool(true));
        assert_eq!(t.root_type(), "Thing");
    }

    #[test]
    fn byte_arrays_and_errors() {
        let t = tree(&[
            (0, "Blob", "Base", 0),
            (1, "vector", "data", 0),
            (2, "Array", "Array", ALIGN_BYTES),
            (3, "int", "size", 0),
            (3, "UInt8", "data", 0),
            (1, "int", "after", 0),
        ]);
        let mut d = 2i32.to_le_bytes().to_vec();
        d.extend_from_slice(&[7, 8, 0, 0]);
        d.extend_from_slice(&5i32.to_le_bytes());
        let v = t.read(&mut Reader::new(&d)).unwrap();
        assert_eq!(v.get("data").as_bytes(), Some(&[7u8, 8][..]));
        assert_eq!(v.get("after").i64(), 5);
        // Truncated data and absurd lengths are errors, not panics.
        assert!(t.read(&mut Reader::new(&d[..6])).is_err());
        assert!(t.read(&mut Reader::new(&[0xff, 0xff, 0xff, 0x7f])).is_err());
    }

    #[test]
    fn generator_string_arrays() {
        // `string[] levels` as emitted by TypeTreeGenerator.
        let t = tree(&[
            (0, "M", "Base", 0),
            (1, "string", "levels", 0),
            (2, "Array", "Array", ALIGN_BYTES),
            (3, "int", "size", 0),
            (3, "string", "data", 0),
            (4, "Array", "Array", ALIGN_BYTES),
            (5, "int", "size", 0),
            (5, "char", "data", 0),
        ]);
        let mut d = 2i32.to_le_bytes().to_vec();
        for s in ["a", "bcde"] {
            d.extend_from_slice(&(s.len() as i32).to_le_bytes());
            d.extend_from_slice(s.as_bytes());
            d.resize(d.len().next_multiple_of(4), 0);
        }
        let mut r = Reader::new(&d);
        let v = t.read(&mut r).unwrap();
        assert_eq!(r.pos(), d.len());
        let l: Vec<&str> = v.get("levels").as_array().iter().map(|s| s.str()).collect();
        assert_eq!(l, ["a", "bcde"]);
    }

    #[test]
    fn malformed_trees_are_rejected() {
        let bad = vec![
            RawNode::new(0, "A", "Base", -1, 0),
            RawNode::new(2, "int", "x", 4, 0),
        ];
        assert!(TypeTree::new(bad).is_err());
        assert!(TypeTree::new(vec![]).is_err());
    }
}
