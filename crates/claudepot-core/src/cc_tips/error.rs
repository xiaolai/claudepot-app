use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TipsError {
    #[error("CC binary not found: tried `{path}`")]
    BinaryNotFound { path: String },

    #[error("failed to read CC binary at `{path}`: {source}")]
    BinaryRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read CC global config at `{path}`: {source}")]
    ConfigRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse CC global config at `{path}`: {source}")]
    ConfigParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to read snapshot log at `{path}`: {source}")]
    SnapshotRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write snapshot log at `{path}`: {source}")]
    SnapshotWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("catalog cache I/O at `{path}`: {source}")]
    CatalogIo {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("catalog cache parse error: {source}")]
    CatalogParse {
        #[source]
        source: serde_json::Error,
    },

    #[error("HOME directory not resolvable")]
    NoHome,
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl ErrorCode for TipsError {
    fn code(&self) -> &'static str {
        match self {
            TipsError::BinaryNotFound { .. } => "cc_tips.binary_not_found",
            TipsError::BinaryRead { .. } => "cc_tips.binary_read",
            TipsError::ConfigRead { .. } => "cc_tips.config_read",
            TipsError::ConfigParse { .. } => "cc_tips.config_parse",
            TipsError::SnapshotRead { .. } => "cc_tips.snapshot_read",
            TipsError::SnapshotWrite { .. } => "cc_tips.snapshot_write",
            TipsError::CatalogIo { .. } => "cc_tips.catalog_io",
            TipsError::CatalogParse { .. } => "cc_tips.catalog_parse",
            TipsError::NoHome => "cc_tips.no_home",
        }
    }

    fn params(&self) -> Value {
        // Every payload here is a filesystem path or an I/O / serde
        // failure string. CC's global config is read but never quoted
        // back into these errors, so no credential reaches `detail`.
        match self {
            TipsError::BinaryNotFound { path } => json!({ "path": path }),
            TipsError::BinaryRead { path, source }
            | TipsError::ConfigRead { path, source }
            | TipsError::SnapshotRead { path, source }
            | TipsError::SnapshotWrite { path, source }
            | TipsError::CatalogIo { path, source } => {
                json!({ "path": path, "detail": source.to_string() })
            }
            TipsError::ConfigParse { path, source } => {
                json!({ "path": path, "detail": source.to_string() })
            }
            TipsError::CatalogParse { source } => json!({ "detail": source.to_string() }),
            TipsError::NoHome => json!({}),
        }
    }
}

pub type TipsResult<T> = Result<T, TipsError>;
