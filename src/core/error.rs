use thiserror::Error;

/// Errors returned by `meilisearch-lib` operations.
///
/// Most variants carry a human-readable message. Several variants convert
/// automatically from underlying crate errors via `#[from]`.
#[derive(Debug, Error)]
pub enum Error {
    /// An error propagated from the milli search engine.
    #[error("Milli error: {0}")]
    Milli(#[from] milli::Error),
    /// An error from the heed LMDB wrapper (database I/O).
    #[error("Heed error: {0}")]
    Heed(#[from] milli::heed::Error),
    /// A standard I/O error (file system, paths, etc.).
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    /// A JSON serialization or deserialization error.
    #[error("Serde Json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    /// The requested index does not exist.
    #[error("Index not found: {0}")]
    IndexNotFound(String),
    /// An index with the given UID already exists.
    #[error("Index already exists: {0}")]
    IndexAlreadyExists(String),
    /// The provided index UID contains invalid characters.
    #[error("Invalid index uid: {0}")]
    InvalidIndexUid(String),
    /// The index cannot be deleted because other references are still held.
    #[error("Index is in use and cannot be deleted: {0}")]
    IndexInUse(String),
    /// A catch-all for internal errors that do not fit another variant.
    #[error("Internal error: {0}")]
    Internal(String),

    /// A document with the requested ID was not found.
    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    /// The primary key cannot be changed because the index has documents.
    #[error("Primary key already present; cannot change on a non-empty index")]
    PrimaryKeyAlreadyPresent,

    /// A primary key is required but the index has none and it cannot be inferred.
    #[error("Primary key is required but was not provided and could not be inferred")]
    PrimaryKeyRequired,

    /// An invalid filter expression was provided.
    #[error("Invalid filter expression: {0}")]
    InvalidFilter(String),

    /// An invalid sort expression was provided.
    #[error("Invalid sort expression: {0}")]
    InvalidSort(String),

    /// The requested embedder does not exist in settings.
    #[error("Embedder not found: {0}")]
    EmbedderNotFound(String),

    /// Dump creation failed.
    #[error("Dump creation failed: {0}")]
    DumpFailed(String),

    /// Snapshot creation failed.
    #[error("Snapshot creation failed: {0}")]
    SnapshotFailed(String),

    /// An experimental feature was used but is not enabled.
    #[error("Experimental feature not enabled: {0}")]
    ExperimentalFeatureNotEnabled(String),

    /// Invalid pagination parameters (both offset/limit and page/hitsPerPage set).
    #[error("Invalid pagination: {0}")]
    InvalidPagination(String),
}

/// A specialized `Result` type for `meilisearch-lib` operations.
pub type Result<T> = std::result::Result<T, Error>;
