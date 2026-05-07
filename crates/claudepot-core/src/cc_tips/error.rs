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

pub type TipsResult<T> = Result<T, TipsError>;
