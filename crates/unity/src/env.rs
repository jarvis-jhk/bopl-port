//! A game's data directory: lazily opened SerializedFiles, cross-file PPtr
//! resolution, MonoScript lookup and resource streams (`.resS`/`.resource`).

use std::collections::HashMap;
use std::fs::File;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use memmap2::Mmap;

use crate::bytes::Reader;
use crate::registry::{MONO_BEHAVIOUR, Registry, ScriptKey};
use crate::serialized::{ObjectInfo, SerializedFile};
use crate::typetree::TypeTree;
use crate::value::{PPtr, Value};
use crate::{Error, Result};

/// Class ID of MonoScript.
pub const MONO_SCRIPT: i32 = 115;

/// One opened SerializedFile.
pub struct AssetFile {
    /// File name relative to the data dir, e.g. `level3`, `sharedassets0.assets`.
    pub name: String,
    data: Mmap,
    /// Parsed metadata.
    pub meta: SerializedFile,
    scripts: OnceLock<Vec<Option<ScriptKey>>>,
}

impl AssetFile {
    /// Opens and parses a file.
    pub fn open(path: &Path, name: &str) -> Result<AssetFile> {
        let f = File::open(path).map_err(|e| Error::io(path, e))?;
        // SAFETY: the game install is treated as read-only and not modified
        // while we run; a concurrent modification would be a user error.
        let data = unsafe { Mmap::map(&f) }.map_err(|e| Error::io(path, e))?;
        let meta =
            SerializedFile::parse(&data).map_err(|e| Error::Format(format!("{name}: {e}")))?;
        Ok(AssetFile {
            name: name.to_owned(),
            data,
            meta,
            scripts: OnceLock::new(),
        })
    }

    /// The raw bytes of an object.
    pub fn object_bytes(&self, info: &ObjectInfo) -> &[u8] {
        &self.data[info.byte_start as usize..][..info.byte_size as usize]
    }

    /// Objects of the given class, in file order.
    pub fn objects_of(&self, class_id: i32) -> impl Iterator<Item = &ObjectInfo> {
        self.meta
            .objects
            .iter()
            .filter(move |o| o.class_id == class_id)
    }

    /// Looks up an object by path ID.
    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.meta.object(path_id)
    }
}

/// A slice of a memory-mapped resource file.
#[derive(Clone)]
pub struct StreamSlice {
    map: Arc<Mmap>,
    start: usize,
    len: usize,
}

impl Deref for StreamSlice {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.map[self.start..][..self.len]
    }
}

/// The result of reading one object.
#[derive(Debug)]
pub struct ObjectRead {
    /// The decoded value tree.
    pub value: Value,
    /// Bytes consumed by the type tree.
    pub consumed: usize,
    /// The object's stored size.
    pub size: usize,
    /// Script class for MonoBehaviours.
    pub script: Option<ScriptKey>,
    /// Whether a full layout was known (false: MonoBehaviour header only).
    pub full_layout: bool,
}

impl ObjectRead {
    /// Whether the layout consumed exactly the stored bytes (allowing the
    /// trailing alignment Unity pads objects to).
    pub fn exact(&self) -> bool {
        self.full_layout
            && (self.consumed == self.size || self.consumed.next_multiple_of(4) == self.size)
    }
}

/// A Unity player's data directory (`<Game>_Data`).
pub struct AssetDir {
    data_dir: PathBuf,
    files: RwLock<HashMap<String, Arc<AssetFile>>>,
    streams: RwLock<HashMap<String, Arc<Mmap>>>,
    registry: &'static Registry,
}

impl AssetDir {
    /// Opens a game directory (containing `*_Data`) or a data directory.
    pub fn open(dir: impl AsRef<Path>) -> Result<AssetDir> {
        let dir = dir.as_ref();
        let data_dir = if dir.join("globalgamemanagers").is_file() {
            dir.to_path_buf()
        } else {
            std::fs::read_dir(dir)
                .map_err(|e| Error::io(dir, e))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| {
                    p.is_dir()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.ends_with("_Data"))
                        && p.join("globalgamemanagers").is_file()
                })
                .ok_or_else(|| Error::NotFound(format!("no Unity data dir in {}", dir.display())))?
        };
        Ok(AssetDir {
            data_dir,
            files: Default::default(),
            streams: Default::default(),
            registry: Registry::get(),
        })
    }

    /// The data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The type-tree registry in use.
    pub fn registry(&self) -> &'static Registry {
        self.registry
    }

    /// Names of the SerializedFiles of a player build: `globalgamemanagers`,
    /// `globalgamemanagers.assets`, `resources.assets`, `sharedassetsN.assets`
    /// and `levelN`, sorted naturally.
    pub fn serialized_file_names(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(&self.data_dir)
            .map_err(|e| Error::io(&self.data_dir, e))?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| {
                (n.starts_with("level")
                    || n.starts_with("sharedassets")
                    || n.starts_with("globalgamemanagers")
                    || n == "resources.assets")
                    && !n.ends_with(".resS")
                    && !n.ends_with(".resource")
            })
            .collect();
        names.sort_by_key(|n| natural_key(n));
        Ok(names)
    }

    fn path_of(&self, name: &str) -> PathBuf {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let p = self.data_dir.join(base);
        if p.is_file() {
            p
        } else {
            self.data_dir.join("Resources").join(base)
        }
    }

    /// Opens (or returns the already opened) file `name`. External paths such
    /// as `Library/unity default resources` are mapped into the data dir.
    pub fn file(&self, name: &str) -> Result<Arc<AssetFile>> {
        let key = name.rsplit(['/', '\\']).next().unwrap_or(name);
        if let Some(f) = self.files.read().unwrap().get(key) {
            return Ok(f.clone());
        }
        let path = self.path_of(key);
        if !path.is_file() {
            return Err(Error::NotFound(format!("asset file {name}")));
        }
        let f = Arc::new(AssetFile::open(&path, key)?);
        Ok(self
            .files
            .write()
            .unwrap()
            .entry(key.to_owned())
            .or_insert(f)
            .clone())
    }

    /// The file a PPtr's `file_id` refers to, seen from `from`.
    pub fn file_of(&self, from: &AssetFile, file_id: i32) -> Result<Arc<AssetFile>> {
        if file_id == 0 {
            return self.file(&from.name);
        }
        let ext = from
            .meta
            .externals
            .get(file_id as usize - 1)
            .ok_or_else(|| Error::Format(format!("{}: bad file id {file_id}", from.name)))?;
        self.file(&ext.path)
    }

    /// Resolves a PPtr to its file and path ID (`None` for null pointers).
    pub fn resolve(&self, from: &AssetFile, p: PPtr) -> Result<Option<(Arc<AssetFile>, i64)>> {
        if p.is_null() {
            return Ok(None);
        }
        Ok(Some((self.file_of(from, p.file_id)?, p.path_id)))
    }

    /// Script classes of a file's MonoBehaviour script types (indexed by
    /// `script_type_index`).
    pub fn script_types<'a>(&self, file: &'a AssetFile) -> &'a [Option<ScriptKey>] {
        file.scripts.get_or_init(|| {
            file.meta
                .script_types
                .iter()
                .map(|st| {
                    let f = self.file_of(file, st.file_index).ok()?;
                    let info = f.object(st.path_id)?;
                    if info.class_id != MONO_SCRIPT {
                        return None;
                    }
                    let v = self
                        .read_with(&f, info, self.registry.engine(MONO_SCRIPT)?)
                        .ok()?;
                    Some(ScriptKey::new(
                        v.get("m_AssemblyName").str(),
                        v.get("m_Namespace").str(),
                        v.get("m_ClassName").str(),
                    ))
                })
                .collect()
        })
    }

    /// The script class of a MonoBehaviour object.
    pub fn script_of(&self, file: &AssetFile, info: &ObjectInfo) -> Option<ScriptKey> {
        if info.class_id != MONO_BEHAVIOUR {
            return None;
        }
        let ty = file.meta.types.get(info.type_index as usize)?;
        let idx = usize::try_from(ty.script_type_index).ok()?;
        self.script_types(file).get(idx)?.clone()
    }

    fn read_with(&self, file: &AssetFile, info: &ObjectInfo, tree: &TypeTree) -> Result<Value> {
        let mut r = Reader::new(file.object_bytes(info));
        r.set_big_endian(file.meta.big_endian);
        tree.read(&mut r)
    }

    /// Reads an object with the best layout available.
    pub fn read_object(&self, file: &AssetFile, info: &ObjectInfo) -> Result<ObjectRead> {
        let bytes = file.object_bytes(info);
        let mut r = Reader::new(bytes);
        r.set_big_endian(file.meta.big_endian);
        let mut script = None;
        let (value, full_layout) = if info.class_id == MONO_BEHAVIOUR {
            script = self.script_of(file, info);
            match script.as_ref().and_then(|k| self.registry.script(k)) {
                Some(t) => (t.read_with(&mut r, self.registry)?, true),
                None => (self.registry.mono_header().read(&mut r)?, false),
            }
        } else {
            let t = self.registry.engine(info.class_id).ok_or_else(|| {
                Error::Unsupported(format!("no layout for class {}", info.class_id))
            })?;
            (t.read(&mut r)?, true)
        };
        Ok(ObjectRead {
            value,
            consumed: r.pos(),
            size: bytes.len(),
            script,
            full_layout,
        })
    }

    /// Reads an object by path ID and checks that the layout fit exactly.
    pub fn read(&self, file: &AssetFile, path_id: i64) -> Result<Value> {
        let info = file
            .object(path_id)
            .ok_or_else(|| Error::NotFound(format!("{}: object {path_id}", file.name)))?;
        let o = self.read_object(file, info)?;
        if o.full_layout && !o.exact() {
            return Err(Error::Format(format!(
                "{}:{path_id}: layout read {} of {} bytes",
                file.name, o.consumed, o.size
            )));
        }
        Ok(o.value)
    }

    /// Reads the object a PPtr points to (`None` for null).
    pub fn deref(&self, from: &AssetFile, p: PPtr) -> Result<Option<(Arc<AssetFile>, i64, Value)>> {
        match self.resolve(from, p)? {
            None => Ok(None),
            Some((f, id)) => {
                let v = self.read(&f, id)?;
                Ok(Some((f, id, v)))
            }
        }
    }

    /// Bytes of a resource stream (`m_StreamData` / `m_Resource`). `path` is
    /// the stored path (e.g. `sharedassets0.assets.resS`, or an `archive:/`
    /// path whose last component is used).
    pub fn stream(&self, path: &str, offset: u64, size: u64) -> Result<StreamSlice> {
        let key = path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned();
        let map = {
            let cached = self.streams.read().unwrap().get(&key).cloned();
            match cached {
                Some(m) => m,
                None => {
                    let p = self.path_of(&key);
                    let f = File::open(&p).map_err(|e| Error::io(&p, e))?;
                    // SAFETY: see AssetFile::open.
                    let m = Arc::new(unsafe { Mmap::map(&f) }.map_err(|e| Error::io(&p, e))?);
                    self.streams
                        .write()
                        .unwrap()
                        .entry(key.clone())
                        .or_insert(m)
                        .clone()
                }
            }
        };
        let (start, len) = (offset as usize, size as usize);
        if start.checked_add(len).is_none_or(|e| e > map.len()) {
            return Err(Error::Format(format!(
                "stream {key}: {offset}+{size} out of bounds"
            )));
        }
        Ok(StreamSlice { map, start, len })
    }
}

/// Sort key that orders `level2` before `level10`.
pub fn natural_key(s: &str) -> (String, u64, String) {
    let i = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    let j = s[i..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(s.len(), |j| i + j);
    (
        s[..i].to_owned(),
        s[i..j].parse().unwrap_or(0),
        s[j..].to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec![
            "level10",
            "level2",
            "sharedassets1.assets",
            "level1",
            "globalgamemanagers",
        ];
        v.sort_by_key(|n| natural_key(n));
        assert_eq!(
            v,
            [
                "globalgamemanagers",
                "level1",
                "level2",
                "level10",
                "sharedassets1.assets"
            ]
        );
    }

    /// The game's data dir, if available (tests that need it skip otherwise).
    fn game() -> Option<AssetDir> {
        let dir = std::env::var_os("BOPL_GAME_DIR")?;
        AssetDir::open(dir).ok()
    }

    #[test]
    fn game_files_are_format_22_and_read_exactly() {
        let Some(dir) = game() else {
            eprintln!("skipped: BOPL_GAME_DIR not set or not a Unity game");
            return;
        };
        let ggm = dir.file("globalgamemanagers").unwrap();
        assert_eq!(ggm.meta.format, 22);
        assert_eq!(ggm.meta.unity_version, "2022.3.60f1");
        let bs = ggm.objects_of(141).next().unwrap();
        let scenes = crate::classes::build_scenes(&dir.read(&ggm, bs.path_id).unwrap());
        assert_eq!(scenes.len(), 130);
        // Every object of a level (engine classes and scripts) has a layout
        // that consumes exactly its stored bytes.
        let lv = dir.file("level6").unwrap();
        for info in &lv.meta.objects {
            let r = dir.read_object(&lv, info).unwrap();
            assert!(r.exact(), "level6:{} {:?}", info.path_id, r.script);
        }
    }
}
