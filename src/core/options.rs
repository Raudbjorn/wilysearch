use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg(not(target_pointer_width = "64"))]
compile_error!("wilysearch requires a 64-bit target (default map sizes exceed 32-bit usize)");

/// Configuration options for an embedded [`Meilisearch`](crate::Meilisearch) instance.
///
/// Controls where the LMDB database is stored on disk and the maximum memory
/// map sizes for indexes and the task database.
///
/// # Default values
///
/// | Field | Default |
/// |-------|---------|
/// | `db_path` | `data.ms` |
/// | `max_index_size` | 100 GiB |
/// | `max_task_db_size` | 10 GiB |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeilisearchOptions {
    /// Directory where the LMDB database files are stored.
    pub db_path: PathBuf,
    /// Maximum memory-map size for each index (bytes). Default: 100 GiB.
    pub max_index_size: usize,
    /// Maximum memory-map size for the task database (bytes). Default: 10 GiB.
    pub max_task_db_size: usize,
}

impl Default for MeilisearchOptions {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("data.ms"),
            max_index_size: 100 * 1024 * 1024 * 1024, // 100GB
            max_task_db_size: 10 * 1024 * 1024 * 1024, // 10GB
        }
    }
}
