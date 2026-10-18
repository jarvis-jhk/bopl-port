//! `unity-assets`: a reader for Unity 2022.3 player data (SerializedFile
//! format 22, built without type trees).
//!
//! * [`serialized`]: the SerializedFile container (header, type and object
//!   tables, externals, script types).
//! * [`typetree`]: a generic type-tree interpreter producing [`Value`] trees.
//! * [`registry`]: type trees for engine classes and the game's MonoBehaviour
//!   classes, generated at dev time and embedded (`typetrees/*.json`).
//! * [`env`]: a data directory with lazy file loading, PPtr resolution,
//!   MonoScript lookup and resource streams.
//! * [`classes`]: typed views of the engine classes the importer needs.
//! * [`texture`]: texture decoding to RGBA8; [`audio`]: FSB5 Vorbis to Ogg.

pub mod audio;
pub mod bytes;
pub mod classes;
pub mod common_strings;
pub mod env;
pub mod registry;
pub mod serialized;
pub mod texture;
pub mod typetree;
pub mod value;

pub use env::{AssetDir, AssetFile, ObjectRead};
pub use registry::{Registry, ScriptKey};
pub use serialized::{ObjectInfo, SerializedFile};
pub use value::{Object, PPtr, Value};

use std::path::Path;

/// Errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Read past the end of a buffer.
    #[error("unexpected end of data at {pos} (length {len})")]
    Eof {
        /// Offset that was needed.
        pos: usize,
        /// Buffer length.
        len: usize,
    },
    /// Malformed data.
    #[error("format error: {0}")]
    Format(String),
    /// Valid data this reader does not handle.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A file or object is missing.
    #[error("not found: {0}")]
    NotFound(String),
    /// I/O failure.
    #[error("{path}: {source}")]
    Io {
        /// File involved.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// JSON failure (embedded type trees).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Error {
        Error::Io {
            path: path.display().to_string(),
            source,
        }
    }
}

/// Result alias of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
