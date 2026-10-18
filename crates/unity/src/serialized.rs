//! Unity SerializedFile container (format versions 17..=22).
//!
//! Layout (format 22, as written by Unity 2022.3):
//!
//! * header, big-endian: legacy `u32` metadata size, `u32` file size, `u32`
//!   format version, `u32` data offset, `u8` endianness + 3 reserved bytes,
//!   then (format >= 22) `u32` metadata size, `i64` file size, `i64` data
//!   offset, `i64` reserved;
//! * metadata, in the file's endianness: Unity version (C string), target
//!   platform `i32`, `bool` type-tree flag, the type table, the object table,
//!   the script-type table, the externals, the reference types and a user
//!   information string;
//! * object data at `data_offset + byte_start`.

use crate::bytes::Reader;
use crate::typetree::RawNode;
use crate::{Error, Result};

/// One entry of the type table.
#[derive(Clone, Debug)]
pub struct SerializedType {
    /// Unity class ID (114 = MonoBehaviour).
    pub class_id: i32,
    /// Stripped types carry no data layout.
    pub is_stripped: bool,
    /// Index into [`SerializedFile::script_types`] for MonoBehaviours, else -1.
    pub script_type_index: i16,
    /// Hash of the script's serialized layout (MonoBehaviour only).
    pub script_id: [u8; 16],
    /// Hash of the type's layout.
    pub old_type_hash: [u8; 16],
    /// Embedded type tree (asset bundles; player data files have none).
    pub nodes: Option<Vec<RawNode>>,
    /// For reference types (`[SerializeReference]` payloads): class,
    /// namespace and assembly names (empty otherwise).
    pub class_name: String,
    /// See [`SerializedType::class_name`].
    pub namespace: String,
    /// See [`SerializedType::class_name`].
    pub assembly: String,
}

/// One entry of the object table.
#[derive(Clone, Debug)]
pub struct ObjectInfo {
    /// The object's local identifier (`m_PathID` in PPtrs).
    pub path_id: i64,
    /// Absolute offset of the object's data in the file.
    pub byte_start: u64,
    /// Size of the object's data.
    pub byte_size: u32,
    /// Index into [`SerializedFile::types`].
    pub type_index: u32,
    /// Class ID, copied from the type table for convenience.
    pub class_id: i32,
}

/// A reference to another SerializedFile.
#[derive(Clone, Debug)]
pub struct External {
    /// GUID of the referenced asset (zero for built-in paths).
    pub guid: [u8; 16],
    /// Reference type (0 = serialized file).
    pub kind: i32,
    /// Path as stored, e.g. `sharedassets0.assets` or `Library/unity default resources`.
    pub path: String,
}

impl External {
    /// The file name part of [`External::path`] (what to look for in the data dir).
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

/// A script-type table entry: a PPtr to a MonoScript.
#[derive(Clone, Copy, Debug)]
pub struct ScriptType {
    /// 0 = this file, n = externals[n - 1].
    pub file_index: i32,
    /// Path ID of the MonoScript.
    pub path_id: i64,
}

/// Parsed metadata of a SerializedFile.
#[derive(Clone, Debug)]
pub struct SerializedFile {
    /// Format version (22 for Unity 2022.3).
    pub format: u32,
    /// Unity version string, e.g. `2022.3.60f1`.
    pub unity_version: String,
    /// Build target platform.
    pub target_platform: i32,
    /// Whether the file embeds type trees (never in player builds).
    pub has_type_trees: bool,
    /// Whether object data is big-endian.
    pub big_endian: bool,
    /// Type table.
    pub types: Vec<SerializedType>,
    /// Object table, in file order.
    pub objects: Vec<ObjectInfo>,
    /// MonoScripts referenced by MonoBehaviour types.
    pub script_types: Vec<ScriptType>,
    /// Referenced files; PPtr file index `n` means `externals[n - 1]`.
    pub externals: Vec<External>,
    /// Offset of the object data region.
    pub data_offset: u64,
    /// Reference types (`[SerializeReference]` payload classes), with their
    /// type trees when the file embeds them.
    pub ref_types: Vec<SerializedType>,
}

impl SerializedFile {
    /// Parses the header and metadata of a SerializedFile.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        r.set_big_endian(true);
        let _legacy_meta = r.u32()?;
        let legacy_size = r.u32()?;
        let format = r.u32()?;
        let legacy_offset = r.u32()?;
        if !(17..=22).contains(&format) {
            return Err(Error::Unsupported(format!(
                "SerializedFile format {format}"
            )));
        }
        let big_endian = r.u8()? != 0;
        r.take(3)?;
        let (file_size, data_offset) = if format >= 22 {
            let _meta = r.u32()?;
            let size = r.i64()? as u64;
            let off = r.i64()? as u64;
            r.i64()?;
            (size, off)
        } else {
            (legacy_size as u64, legacy_offset as u64)
        };
        if file_size as usize > data.len() || data_offset > file_size {
            return Err(Error::Format(format!(
                "file size {file_size} / data offset {data_offset} vs {} bytes",
                data.len()
            )));
        }
        r.set_big_endian(big_endian);

        let unity_version = r.cstr()?;
        let target_platform = r.i32()?;
        let has_type_trees = r.u8()? != 0;
        let n = r.len_prefix(4)?;
        let mut types = Vec::with_capacity(n);
        for _ in 0..n {
            types.push(read_type(&mut r, false, has_type_trees, format)?);
        }

        let n = r.len_prefix(20)?;
        let mut objects = Vec::with_capacity(n);
        for _ in 0..n {
            r.align4();
            let path_id = r.i64()?;
            let start = if format >= 22 {
                r.i64()? as u64
            } else {
                r.u32()? as u64
            };
            let byte_size = r.u32()?;
            let type_index = r.i32()?;
            let ty = types
                .get(type_index as usize)
                .ok_or_else(|| Error::Format(format!("object {path_id}: bad type {type_index}")))?;
            let byte_start = data_offset + start;
            if byte_start + byte_size as u64 > data.len() as u64 {
                return Err(Error::Format(format!("object {path_id} out of bounds")));
            }
            objects.push(ObjectInfo {
                path_id,
                byte_start,
                byte_size,
                type_index: type_index as u32,
                class_id: ty.class_id,
            });
        }

        let n = r.len_prefix(12)?;
        let mut script_types = Vec::with_capacity(n);
        for _ in 0..n {
            let file_index = r.i32()?;
            r.align4();
            let path_id = r.i64()?;
            script_types.push(ScriptType {
                file_index,
                path_id,
            });
        }

        let n = r.len_prefix(21)?;
        let mut externals = Vec::with_capacity(n);
        for _ in 0..n {
            let _empty = r.cstr()?;
            let guid: [u8; 16] = r.take(16)?.try_into().unwrap();
            let kind = r.i32()?;
            let path = r.cstr()?;
            externals.push(External { guid, kind, path });
        }

        let mut ref_types = Vec::new();
        if format >= 20 {
            let n = r.len_prefix(4)?;
            for _ in 0..n {
                ref_types.push(read_type(&mut r, true, has_type_trees, format)?);
            }
        }
        // Followed by the user information string, which we do not need.

        Ok(SerializedFile {
            format,
            unity_version,
            target_platform,
            has_type_trees,
            big_endian,
            types,
            objects,
            script_types,
            externals,
            data_offset,
            ref_types,
        })
    }

    /// Finds an object by path ID (binary search: the table is sorted).
    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        match self.objects.binary_search_by_key(&path_id, |o| o.path_id) {
            Ok(i) => Some(&self.objects[i]),
            Err(_) => self.objects.iter().find(|o| o.path_id == path_id),
        }
    }
}

fn read_type(
    r: &mut Reader,
    is_ref_type: bool,
    trees: bool,
    format: u32,
) -> Result<SerializedType> {
    let class_id = r.i32()?;
    let is_stripped = r.u8()? != 0;
    let script_type_index = r.i16()?;
    let mut script_id = [0; 16];
    if (is_ref_type && script_type_index >= 0) || class_id == 114 {
        script_id.copy_from_slice(r.take(16)?);
    }
    let old_type_hash: [u8; 16] = r.take(16)?.try_into().unwrap();
    let mut t = SerializedType {
        class_id,
        is_stripped,
        script_type_index,
        script_id,
        old_type_hash,
        nodes: None,
        class_name: String::new(),
        namespace: String::new(),
        assembly: String::new(),
    };
    // Without embedded type trees there are no node blobs or dependency lists.
    if trees {
        t.nodes = Some(read_tree_blob(r, format)?);
        if format >= 21 {
            if is_ref_type {
                t.class_name = r.cstr()?;
                t.namespace = r.cstr()?;
                t.assembly = r.cstr()?;
            } else {
                let n = r.len_prefix(4)?;
                for _ in 0..n {
                    r.i32()?;
                }
            }
        }
    }
    Ok(t)
}

/// A type tree in blob form (format >= 12): node records, then a string
/// buffer; string offsets with the high bit set index Unity's built-in
/// table ([`crate::common_strings`]).
fn read_tree_blob(r: &mut Reader, format: u32) -> Result<Vec<RawNode>> {
    let n = r.len_prefix(24)?;
    let buf_len = r.len_prefix(1)?;
    let mut recs = Vec::with_capacity(n);
    for _ in 0..n {
        let _version = r.u16()?;
        let level = r.u8()?;
        let _flags = r.u8()?;
        let type_off = r.u32()?;
        let name_off = r.u32()?;
        let byte_size = r.i32()?;
        let _index = r.i32()?;
        let meta = r.u32()?;
        if format >= 19 {
            r.u64()?;
        }
        recs.push((level, type_off, name_off, byte_size, meta));
    }
    let buf = r.take(buf_len)?;
    let string = |off: u32| -> Result<String> {
        if off & 0x8000_0000 != 0 {
            return crate::common_strings::common_string(off & 0x7fff_ffff)
                .map(String::from)
                .ok_or_else(|| Error::Format(format!("unknown common string {off:#x}")));
        }
        let rest = buf
            .get(off as usize..)
            .ok_or_else(|| Error::Format(format!("type tree string {off} out of range")))?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    recs.into_iter()
        .map(|(level, t, nm, size, meta)| {
            Ok(RawNode {
                level,
                type_name: string(t)?,
                name: string(nm)?,
                byte_size: size,
                meta,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal format-22 file with one object of class 1.
    pub(crate) fn tiny_file(payload: &[u8]) -> Vec<u8> {
        let mut meta = Vec::new();
        meta.extend_from_slice(b"2022.3.60f1\0");
        meta.extend_from_slice(&19i32.to_le_bytes());
        meta.push(0); // no type trees
        meta.extend_from_slice(&1i32.to_le_bytes()); // one type
        meta.extend_from_slice(&1i32.to_le_bytes()); // class 1
        meta.push(0);
        meta.extend_from_slice(&(-1i16).to_le_bytes());
        meta.extend_from_slice(&[0; 16]);
        meta.extend_from_slice(&1i32.to_le_bytes()); // one object
        while (48 + meta.len()) % 4 != 0 {
            meta.push(0);
        }
        meta.extend_from_slice(&7i64.to_le_bytes());
        meta.extend_from_slice(&0i64.to_le_bytes());
        meta.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        meta.extend_from_slice(&0i32.to_le_bytes());
        meta.extend_from_slice(&0i32.to_le_bytes()); // scripts
        meta.extend_from_slice(&1i32.to_le_bytes()); // externals
        meta.push(0);
        meta.extend_from_slice(&[0; 16]);
        meta.extend_from_slice(&0i32.to_le_bytes());
        meta.extend_from_slice(b"Library/unity default resources\0");
        meta.extend_from_slice(&0i32.to_le_bytes()); // ref types
        meta.push(0); // user info
        let data_offset = (48 + meta.len()).next_multiple_of(16);
        let size = data_offset + payload.len();
        let mut f = Vec::new();
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&22u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(&(meta.len() as u32).to_be_bytes());
        f.extend_from_slice(&(size as i64).to_be_bytes());
        f.extend_from_slice(&(data_offset as i64).to_be_bytes());
        f.extend_from_slice(&0i64.to_be_bytes());
        f.extend_from_slice(&meta);
        f.resize(data_offset, 0);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn parses_minimal_file() {
        let f = tiny_file(&[1, 2, 3, 4]);
        let sf = SerializedFile::parse(&f).unwrap();
        assert_eq!(sf.format, 22);
        assert_eq!(sf.unity_version, "2022.3.60f1");
        assert_eq!(sf.objects.len(), 1);
        let o = sf.object(7).unwrap();
        assert_eq!(o.class_id, 1);
        assert_eq!(&f[o.byte_start as usize..][..4], &[1, 2, 3, 4]);
        assert_eq!(sf.externals[0].file_name(), "unity default resources");
    }

    #[test]
    fn rejects_garbage() {
        assert!(SerializedFile::parse(&[0; 64]).is_err());
        assert!(SerializedFile::parse(&[1, 2]).is_err());
    }
}
