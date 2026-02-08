use milli::progress::EmbedderStats;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use crate::core::error::{Error, Result};
use crate::core::index::Index;
use crate::core::options::MeilisearchOptions;
use crate::core::search::{
    ComputedFacets, FederatedMultiSearchQuery, FederatedSearchResult, Federation, HitsInfo,
    MultiSearchQuery, MultiSearchResult, SearchResultWithIndex,
};
use crate::core::vector::VectorStore;

/// Statistics for a single index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    /// Number of documents in the index.
    pub number_of_documents: u64,
    /// Whether the index is currently loaded in memory.
    pub is_indexing: bool,
    /// Distribution of fields across documents (field name -> count).
    pub field_distribution: BTreeMap<String, u64>,
    /// The primary key field, if set.
    pub primary_key: Option<String>,
}

/// Health status of the embedded Meilisearch instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Status string, typically `"available"`.
    pub status: String,
}

/// Version information for the embedded Meilisearch engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// Git commit SHA (or `"embedded"` for the library build).
    pub commit_sha: String,
    /// Git commit date (or `"embedded"` for the library build).
    pub commit_date: String,
    /// Crate version from `Cargo.toml`.
    pub pkg_version: String,
}

/// Aggregate statistics across all indexes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalStats {
    /// Total size of the database directory in bytes.
    pub database_size: u64,
    /// ISO-8601 timestamp of the last document or settings update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
    /// Per-index statistics keyed by index UID.
    pub indexes: HashMap<String, IndexStats>,
}

/// Information about a dump operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpInfo {
    /// Unique identifier for this dump.
    pub uid: String,
    /// Filesystem path where the dump was written.
    pub path: String,
}

/// Runtime-togglable experimental feature flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatures {
    /// Enable Prometheus metrics endpoint.
    #[serde(default)]
    pub metrics: bool,
    /// Enable the logs route for real-time log streaming.
    #[serde(default)]
    pub logs_route: bool,
    /// Enable editing documents by function (JavaScript runtime).
    #[serde(default)]
    pub edit_documents_by_function: bool,
    /// Enable the `CONTAINS` filter operator.
    #[serde(default)]
    pub contains_filter: bool,
    /// Enable composite (multi-source) embedders.
    #[serde(default)]
    pub composite_embedders: bool,
    /// Enable multimodal embeddings.
    #[serde(default)]
    pub multimodal: bool,
    /// Enable the vector store settings in index configuration.
    #[serde(default)]
    pub vector_store_setting: bool,
}

/// Enhanced index information with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    /// The unique identifier for this index.
    pub uid: String,
    /// The primary key field, if set.
    pub primary_key: Option<String>,
    /// ISO-8601 timestamp when the index was created.
    pub created_at: Option<String>,
    /// ISO-8601 timestamp when the index was last updated.
    pub updated_at: Option<String>,
}

/// Per-index creation and update timestamps, persisted as `metadata.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub created_at: String,
    pub updated_at: String,
}

const METADATA_FILE: &str = "metadata.json";

fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn load_index_metadata(index_dir: &Path) -> Option<IndexMetadata> {
    let path = index_dir.join(METADATA_FILE);
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_index_metadata(index_dir: &Path, meta: &IndexMetadata) -> Result<()> {
    let path = index_dir.join(METADATA_FILE);
    let data = serde_json::to_string(meta)?;
    std::fs::write(&path, data)?;
    Ok(())
}

/// Top-level handle for an embedded Meilisearch instance.
///
/// `Meilisearch` manages an in-memory cache of loaded indexes, each with its
/// own LMDB environment. Indexes are created, opened, and deleted through this
/// struct.
///
/// Thread safety: `Meilisearch` is `Send + Sync` and can be shared via `Arc`.
pub struct Meilisearch {
    options: MeilisearchOptions,
    indexes: RwLock<HashMap<String, Arc<Index>>>,
    index_metadata: RwLock<HashMap<String, IndexMetadata>>,
    vector_store: Option<Arc<dyn VectorStore>>,
    experimental_features: RwLock<ExperimentalFeatures>,
}

impl Meilisearch {
    /// Create a new embedded Meilisearch instance with the given options.
    ///
    /// Creates the database directory at `options.db_path` if it does not exist
    /// and loads persisted index metadata.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use wilysearch::core::{Meilisearch, MeilisearchOptions};
    ///
    /// let meili = Meilisearch::new(MeilisearchOptions {
    ///     db_path: "/tmp/my-search-db".into(),
    ///     ..Default::default()
    /// })?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the database directory cannot be created.
    pub fn new(options: MeilisearchOptions) -> Result<Self> {
        std::fs::create_dir_all(&options.db_path)?;

        // Load persisted metadata for all existing indexes, backfilling any that lack it
        let mut metadata_cache = HashMap::new();
        let indexes_dir = options.db_path.join("indexes");
        if indexes_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&indexes_dir) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        if let Some(uid) = entry.file_name().to_str() {
                            let uid = uid.to_string();
                            let meta = load_index_metadata(&entry.path()).unwrap_or_else(|| {
                                let now = now_iso8601();
                                let backfill = IndexMetadata {
                                    created_at: now.clone(),
                                    updated_at: now,
                                };
                                let _ = save_index_metadata(&entry.path(), &backfill);
                                backfill
                            });
                            metadata_cache.insert(uid, meta);
                        }
                    }
                }
            }
        }

        Ok(Self {
            options,
            indexes: RwLock::new(HashMap::new()),
            index_metadata: RwLock::new(metadata_cache),
            vector_store: None,
            experimental_features: RwLock::new(ExperimentalFeatures::default()),
        })
    }

    /// Attach an external vector store for hybrid/vector search support.
    pub fn with_vector_store(mut self, vector_store: Arc<dyn VectorStore>) -> Self {
        self.vector_store = Some(vector_store);
        self
    }

    /// Create a new index with the given UID and optional primary key.
    ///
    /// The UID must contain only ASCII alphanumeric characters, hyphens, or
    /// underscores. The index is immediately usable after creation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default())?;
    /// let movies = meili.create_index("movies", Some("id"))?;
    /// let logs = meili.create_index("logs", None)?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidIndexUid`] if the UID contains invalid characters.
    /// - [`Error::IndexAlreadyExists`] if an index with this UID already exists.
    pub fn create_index(&self, uid: &str, primary_key: Option<&str>) -> Result<Arc<Index>> {
        // Validation
        if !is_valid_uid(uid) {
            return Err(Error::InvalidIndexUid(uid.to_string()));
        }

        let mut indexes = self.indexes.write().unwrap_or_else(|e| e.into_inner());

        if indexes.contains_key(uid) {
            return Err(Error::IndexAlreadyExists(uid.to_string()));
        }

        let index_path = self.options.db_path.join("indexes").join(uid);
        std::fs::create_dir_all(&index_path)?;

        // Use read_txn_without_tls() to get EnvOpenOptions<WithoutTls>
        let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
        options.map_size(self.options.max_index_size);

        // milli::Index::new(options, path, creation_bool)
        let milli_index = milli::Index::new(options, &index_path, true).map_err(Error::Milli)?;

        if let Some(pk) = primary_key {
            let mut wtxn = milli_index.write_txn().map_err(Error::Heed)?;

            let embedder_stats = Arc::new(EmbedderStats::default());
            let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
            let indexer_config = milli::update::IndexerConfig::default();

            let mut builder = milli::update::Settings::new(
                &mut wtxn,
                &milli_index,
                &indexer_config,
            );
            builder.set_primary_key(pk.to_string());

            // execute(must_stop_processing, progress, ip_policy, embedder_stats)
            builder
                .execute(
                    &|| false,
                    &milli::progress::Progress::default(),
                    &ip_policy,
                    embedder_stats,
                )
                .map_err(Error::Milli)?;

            wtxn.commit().map_err(Error::Heed)?;
        }

        let index = Arc::new(Index::new(milli_index, self.vector_store.clone()));
        indexes.insert(uid.to_string(), index.clone());

        // Persist creation metadata
        let now = now_iso8601();
        let meta = IndexMetadata {
            created_at: now.clone(),
            updated_at: now,
        };
        let _ = save_index_metadata(&index_path, &meta);
        self.index_metadata
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uid.to_string(), meta);

        Ok(index)
    }

    /// Get an index by UID, loading it from disk if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`Error::IndexNotFound`] if no index with the given UID exists.
    pub fn get_index(&self, uid: &str) -> Result<Arc<Index>> {
        let indexes = self.indexes.read().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = indexes.get(uid) {
            return Ok(index.clone());
        }

        // Try to load it if it exists on disk
        let index_path = self.options.db_path.join("indexes").join(uid);
        if index_path.exists() {
            drop(indexes); // Drop read lock before acquiring write lock
            let mut indexes = self.indexes.write().unwrap_or_else(|e| e.into_inner());

            // Check again in case someone else loaded it
            if let Some(index) = indexes.get(uid) {
                return Ok(index.clone());
            }

            let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
            options.map_size(self.options.max_index_size);

            // milli::Index::new(options, path, creation_bool)
            let milli_index =
                milli::Index::new(options, &index_path, false).map_err(Error::Milli)?;
            let index = Arc::new(Index::new(milli_index, self.vector_store.clone()));
            indexes.insert(uid.to_string(), index.clone());
            return Ok(index);
        }

        Err(Error::IndexNotFound(uid.to_string()))
    }

    /// Delete an index and all its data from disk.
    ///
    /// This will:
    /// 1. Remove the index from the in-memory cache if loaded
    /// 2. Delete all index files from disk
    ///
    /// Returns an error if:
    /// - The index does not exist
    /// - The index is still in use (has other references)
    pub fn delete_index(&self, uid: &str) -> Result<()> {
        let index_path = self.options.db_path.join("indexes").join(uid);

        // Check if index exists on disk
        if !index_path.exists() {
            return Err(Error::IndexNotFound(uid.to_string()));
        }

        // Remove from memory cache if present
        {
            let mut indexes = self.indexes.write().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = indexes.remove(uid) {
                // Check if there are other references to this index
                // Arc::strong_count of 1 means we hold the only reference
                if Arc::strong_count(&index) > 1 {
                    // Put it back and return error
                    indexes.insert(uid.to_string(), index);
                    return Err(Error::IndexInUse(uid.to_string()));
                }
                // Drop the index to close LMDB environment before deleting files
                drop(index);
            }
        }

        // Delete index directory from disk (metadata.json goes with it)
        std::fs::remove_dir_all(&index_path)?;

        // Remove from metadata cache
        self.index_metadata
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(uid);

        Ok(())
    }

    /// List all index UIDs.
    ///
    /// Returns UIDs for all indexes that exist on disk, regardless of
    /// whether they are currently loaded in memory.
    pub fn list_indexes(&self) -> Result<Vec<String>> {
        let indexes_dir = self.options.db_path.join("indexes");

        if !indexes_dir.exists() {
            return Ok(Vec::new());
        }

        let mut uids = Vec::new();
        for entry in std::fs::read_dir(&indexes_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    uids.push(name.to_string());
                }
            }
        }

        uids.sort();
        Ok(uids)
    }

    /// Check if an index exists.
    ///
    /// Returns `true` if the index exists on disk (does not require loading).
    pub fn index_exists(&self, uid: &str) -> bool {
        let index_path = self.options.db_path.join("indexes").join(uid);
        index_path.exists() && index_path.is_dir()
    }

    /// Get statistics for an index.
    ///
    /// This will load the index if it's not already in memory.
    pub fn index_stats(&self, uid: &str) -> Result<IndexStats> {
        let index = self.get_index(uid)?;

        let rtxn = index.inner.read_txn().map_err(Error::Heed)?;

        let number_of_documents = index.inner.number_of_documents(&rtxn).map_err(Error::Milli)?;
        let field_distribution = index.inner.field_distribution(&rtxn).map_err(Error::Heed)?;
        let primary_key = index.inner.primary_key(&rtxn).map_err(Error::Heed)?.map(String::from);

        Ok(IndexStats {
            number_of_documents,
            is_indexing: false, // We don't track this currently
            field_distribution,
            primary_key,
        })
    }

    /// Update the `updated_at` timestamp for an index and persist to disk.
    pub fn touch_index_updated(&self, uid: &str) -> Result<()> {
        let now = now_iso8601();
        let index_path = self.options.db_path.join("indexes").join(uid);
        let mut cache = self.index_metadata.write().unwrap_or_else(|e| e.into_inner());
        if let Some(meta) = cache.get_mut(uid) {
            meta.updated_at = now;
            save_index_metadata(&index_path, meta)?;
        }
        Ok(())
    }

    /// Get a clone of the metadata for an index, if it exists.
    pub fn get_index_metadata(&self, uid: &str) -> Option<IndexMetadata> {
        self.index_metadata
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(uid)
            .cloned()
    }

    // ========================================================================
    // Instance Operations (Tasks 5.1 - 5.4)
    // ========================================================================

    /// Check if the embedded instance is operational.
    pub fn health(&self) -> HealthStatus {
        HealthStatus { status: "available".to_string() }
    }

    /// Get version information for the embedded Meilisearch engine.
    pub fn version(&self) -> VersionInfo {
        VersionInfo {
            commit_sha: "embedded".to_string(),
            commit_date: "embedded".to_string(),
            pkg_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Get aggregate statistics across all indexes.
    pub fn stats(&self) -> Result<GlobalStats> {
        let index_uids = self.list_indexes()?;
        let mut indexes = HashMap::new();
        let mut total_size = 0u64;

        for uid in &index_uids {
            if let Ok(stats) = self.index_stats(uid) {
                indexes.insert(uid.clone(), stats);
            }
        }

        // Calculate database size from the db_path directory
        let db_path = &self.options.db_path;
        if db_path.exists() {
            total_size = dir_size(db_path).unwrap_or(0);
        }

        let last_update = self
            .index_metadata
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|m| m.updated_at.clone())
            .max();

        Ok(GlobalStats {
            database_size: total_size,
            last_update,
            indexes,
        })
    }

    /// Create a database dump at the specified directory.
    pub fn create_dump(&self, dump_dir: &std::path::Path) -> Result<DumpInfo> {
        let uid = Uuid::new_v4().to_string();
        let dump_path = dump_dir.join(&uid);
        std::fs::create_dir_all(&dump_path)?;

        let index_uids = self.list_indexes()?;
        for index_uid in &index_uids {
            let index = self.get_index(index_uid)?;
            let index_dump_dir = dump_path.join(index_uid);
            std::fs::create_dir_all(&index_dump_dir)?;

            // Export settings
            let settings = index.get_settings()?;
            let settings_json = serde_json::to_string_pretty(&settings)?;
            std::fs::write(index_dump_dir.join("settings.json"), settings_json)?;

            // Export documents in batches to avoid OOM on large indexes
            let docs_file = std::fs::File::create(index_dump_dir.join("documents.json"))?;
            let mut writer = std::io::BufWriter::new(docs_file);
            use std::io::Write;
            write!(writer, "[")?;

            const BATCH_SIZE: usize = 1000;
            let mut offset = 0;
            let mut first = true;
            loop {
                let batch = index.get_documents(offset, BATCH_SIZE)?;
                for doc in &batch.documents {
                    if !first {
                        write!(writer, ",")?;
                    }
                    first = false;
                    serde_json::to_writer_pretty(&mut writer, doc)?;
                }
                if batch.documents.len() < BATCH_SIZE {
                    break;
                }
                offset += BATCH_SIZE;
            }
            write!(writer, "]")?;
            writer.flush()?;
        }

        Ok(DumpInfo {
            uid,
            path: dump_path.to_string_lossy().to_string(),
        })
    }

    /// Export selected indexes to a filesystem directory.
    ///
    /// - `export_path` — destination directory (created if absent)
    /// - `indexes` — `None` exports all; `Some(map)` filters to those UIDs.
    ///   The bool value controls whether `settings.json` is written (`true` = include).
    pub fn export(
        &self,
        export_path: &std::path::Path,
        indexes: Option<&HashMap<String, bool>>,
    ) -> Result<()> {
        std::fs::create_dir_all(export_path)?;

        let all_uids = self.list_indexes()?;
        let uids_to_export: Vec<&str> = match indexes {
            Some(map) => all_uids.iter().filter(|uid| map.contains_key(*uid)).map(|s| s.as_str()).collect(),
            None => all_uids.iter().map(|s| s.as_str()).collect(),
        };

        for uid in &uids_to_export {
            let index = self.get_index(uid)?;
            let index_dir = export_path.join(uid);
            std::fs::create_dir_all(&index_dir)?;

            // Write settings if requested (default: true)
            let include_settings = indexes
                .and_then(|m| m.get(*uid))
                .copied()
                .unwrap_or(true);
            if include_settings {
                let settings = index.get_settings()?;
                let settings_json = serde_json::to_string_pretty(&settings)?;
                std::fs::write(index_dir.join("settings.json"), settings_json)?;
            }

            // Export documents in batches (same logic as create_dump)
            let docs_file = std::fs::File::create(index_dir.join("documents.json"))?;
            let mut writer = std::io::BufWriter::new(docs_file);
            use std::io::Write;
            write!(writer, "[")?;

            const BATCH_SIZE: usize = 1000;
            let mut offset = 0;
            let mut first = true;
            loop {
                let batch = index.get_documents(offset, BATCH_SIZE)?;
                for doc in &batch.documents {
                    if !first {
                        write!(writer, ",")?;
                    }
                    first = false;
                    serde_json::to_writer_pretty(&mut writer, doc)?;
                }
                if batch.documents.len() < BATCH_SIZE {
                    break;
                }
                offset += BATCH_SIZE;
            }
            write!(writer, "]")?;
            writer.flush()?;
        }

        Ok(())
    }

    /// Create a snapshot of the database at the specified directory.
    ///
    /// Uses LMDB's built-in copy API to produce a consistent snapshot
    /// without risking corruption from copying live data files.
    pub fn create_snapshot(&self, snapshot_dir: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(snapshot_dir)?;

        let indexes_dir = self.options.db_path.join("indexes");
        if indexes_dir.exists() {
            let snapshot_indexes = snapshot_dir.join("indexes");
            std::fs::create_dir_all(&snapshot_indexes)?;

            for entry in std::fs::read_dir(&indexes_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let name = entry.file_name();
                    let dest = snapshot_indexes.join(&name);
                    std::fs::create_dir_all(&dest)?;

                    // Use LMDB's copy_to_file for a consistent snapshot of live data
                    let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
                    options.map_size(self.options.max_index_size);

                    let env = unsafe {
                        options.open(entry.path()).map_err(Error::Heed)?
                    };
                    let snapshot_file = dest.join("data.mdb");
                    let mut file = std::fs::File::create(&snapshot_file)?;
                    env.copy_to_file(&mut file, milli::heed::CompactionOption::Disabled)
                        .map_err(Error::Heed)?;
                }
            }
        }

        Ok(())
    }

    /// Get the current experimental feature flags.
    pub fn get_experimental_features(&self) -> ExperimentalFeatures {
        self.experimental_features.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Update experimental feature flags.
    pub fn update_experimental_features(&self, features: ExperimentalFeatures) -> ExperimentalFeatures {
        let mut current = self.experimental_features.write().unwrap_or_else(|e| e.into_inner());
        *current = features;
        current.clone()
    }

    // ========================================================================
    // Index Operations (Tasks 4.1 - 4.3)
    // ========================================================================

    /// Atomically swap the contents of pairs of indexes.
    pub fn swap_indexes(&self, swaps: &[(&str, &str)]) -> Result<()> {
        // Validate all indexes exist
        for (a, b) in swaps {
            if !self.index_exists(a) {
                return Err(Error::IndexNotFound(a.to_string()));
            }
            if !self.index_exists(b) {
                return Err(Error::IndexNotFound(b.to_string()));
            }
        }

        let mut indexes = self.indexes.write().unwrap_or_else(|e| e.into_inner());

        for (a, b) in swaps {
            let index_path_a = self.options.db_path.join("indexes").join(a);
            let index_path_b = self.options.db_path.join("indexes").join(b);
            let tmp_path = self.options.db_path.join("indexes").join(format!("_swap_tmp_{}", Uuid::new_v4()));

            // Remove from memory cache to close LMDB environments
            let idx_a = indexes.remove(*a);
            let idx_b = indexes.remove(*b);
            drop(idx_a);
            drop(idx_b);

            // Rename directories atomically (metadata.json moves with them)
            std::fs::rename(&index_path_a, &tmp_path)?;
            std::fs::rename(&index_path_b, &index_path_a)?;
            std::fs::rename(&tmp_path, &index_path_b)?;

            // Swap metadata entries and bump updated_at
            let now = now_iso8601();
            let mut meta_cache = self.index_metadata.write().unwrap_or_else(|e| e.into_inner());
            let meta_a = meta_cache.remove(*a);
            let meta_b = meta_cache.remove(*b);
            if let Some(mut m) = meta_b {
                m.updated_at = now.clone();
                let _ = save_index_metadata(&index_path_a, &m);
                meta_cache.insert(a.to_string(), m);
            }
            if let Some(mut m) = meta_a {
                m.updated_at = now;
                let _ = save_index_metadata(&index_path_b, &m);
                meta_cache.insert(b.to_string(), m);
            }
        }

        // Indexes will be re-loaded on next access via get_index()
        Ok(())
    }

    /// Compact an index to reclaim disk space after bulk deletions.
    ///
    /// This triggers an LMDB copy-compact on the index's environment, creating a
    /// compacted copy and replacing the original. The index is unloaded from the
    /// in-memory cache so the next access re-opens it from the compacted files.
    ///
    /// # Errors
    ///
    /// Returns [`Error::IndexNotFound`] if the index does not exist.
    pub fn compact_index(&self, uid: &str) -> Result<()> {
        let index_path = self.options.db_path.join("indexes").join(uid);
        if !index_path.exists() {
            return Err(Error::IndexNotFound(uid.to_string()));
        }

        // Remove from cache so we can close the LMDB environment
        {
            let mut indexes = self.indexes.write().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = indexes.remove(uid) {
                // Check if there are other references to this index
                // Arc::strong_count of 1 means we hold the only reference
                if Arc::strong_count(&index) > 1 {
                    // Put it back and return error
                    indexes.insert(uid.to_string(), index);
                    return Err(Error::IndexInUse(uid.to_string()));
                }
                drop(index);
            }
        }

        // Perform copy-compact: open env, copy compact to temp, replace original
        let tmp_path = self.options.db_path.join("indexes").join(format!("_compact_tmp_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_path)?;

        {
            let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
            options.map_size(self.options.max_index_size);

            let env = unsafe { options.open(&index_path).map_err(Error::Heed)? };
            let compact_path = tmp_path.join("data.mdb");
            let mut file = std::fs::File::create(&compact_path)?;
            env.copy_to_file(&mut file, milli::heed::CompactionOption::Enabled)
                .map_err(Error::Heed)?;
        }

        // Replace original data.mdb with the compacted one
        let original_data = index_path.join("data.mdb");
        let compacted_data = tmp_path.join("data.mdb");
        if compacted_data.exists() {
            std::fs::rename(&compacted_data, &original_data)?;
        }

        // Clean up temp directory
        let _ = std::fs::remove_dir_all(&tmp_path);

        Ok(())
    }

    /// List indexes with pagination and metadata.
    pub fn list_indexes_with_pagination(&self, offset: usize, limit: usize) -> Result<(Vec<IndexInfo>, usize)> {
        let all_uids = self.list_indexes()?;
        let total = all_uids.len();

        let paginated: Vec<String> = all_uids.into_iter().skip(offset).take(limit).collect();

        let meta_cache = self.index_metadata.read().unwrap_or_else(|e| e.into_inner());
        let mut infos = Vec::with_capacity(paginated.len());
        for uid in paginated {
            let primary_key = if let Ok(index) = self.get_index(&uid) {
                index.primary_key().ok().flatten()
            } else {
                None
            };

            let (created_at, updated_at) = meta_cache
                .get(&uid)
                .map(|m| (Some(m.created_at.clone()), Some(m.updated_at.clone())))
                .unwrap_or((None, None));

            infos.push(IndexInfo {
                uid,
                primary_key,
                created_at,
                updated_at,
            });
        }

        Ok((infos, total))
    }

    // ========================================================================
    // Multi-Search Operations
    // ========================================================================

    /// Execute multiple search queries across different indexes in a single call.
    ///
    /// Each query targets a specific index identified by `index_uid`. Results are
    /// returned in the same order as the input queries, each tagged with the
    /// index UID it came from.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// use wilysearch::core::search::{MultiSearchQuery, SearchQuery};
    ///
    /// let queries = vec![
    ///     MultiSearchQuery {
    ///         index_uid: "movies".to_string(),
    ///         query: SearchQuery::new("action"),
    ///     },
    ///     MultiSearchQuery {
    ///         index_uid: "books".to_string(),
    ///         query: SearchQuery::new("thriller"),
    ///     },
    /// ];
    ///
    /// let result = meili.multi_search(queries)?;
    /// for r in &result.results {
    ///     println!("{}: {} hits", r.index_uid, r.result.hits.len());
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn multi_search(&self, queries: Vec<MultiSearchQuery>) -> Result<MultiSearchResult> {
        let mut results = Vec::with_capacity(queries.len());

        for msq in queries {
            let index = self.get_index(&msq.index_uid)?;
            let result = index.search(&msq.query)?;
            results.push(SearchResultWithIndex {
                index_uid: msq.index_uid,
                result,
            });
        }

        Ok(MultiSearchResult { results })
    }

    /// Execute a federated multi-search that merges hits from multiple indexes.
    ///
    /// Unlike [`multi_search`](Self::multi_search), federated search returns a
    /// single flat list of hits drawn from all queried indexes. Each query can
    /// carry optional [`FederationOptions`](crate::FederationOptions) (weight, query_position) that
    /// influence how its hits are ranked relative to hits from other queries.
    ///
    /// The `federation` parameter controls global result pagination
    /// (limit/offset or page/hits_per_page) and facet aggregation.
    ///
    /// **Note:** This is a basic implementation that concatenates hits from each
    /// query, sorts by ranking score when available, and applies the federation
    /// pagination. Full score normalization and cross-index ranking are not yet
    /// implemented.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// use wilysearch::core::search::{
    ///     FederatedMultiSearchQuery, Federation, FederationOptions, SearchQuery,
    /// };
    ///
    /// let queries = vec![
    ///     FederatedMultiSearchQuery {
    ///         index_uid: "movies".to_string(),
    ///         query: SearchQuery::new("action"),
    ///         federation_options: Some(FederationOptions { weight: 1.0, query_position: None }),
    ///     },
    ///     FederatedMultiSearchQuery {
    ///         index_uid: "books".to_string(),
    ///         query: SearchQuery::new("thriller"),
    ///         federation_options: Some(FederationOptions { weight: 0.8, query_position: None }),
    ///     },
    /// ];
    ///
    /// let federation = Federation { limit: 10, offset: 0, ..Default::default() };
    /// let result = meili.multi_search_federated(queries, federation)?;
    /// println!("Total merged hits: {}", result.hits.len());
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn multi_search_federated(
        &self,
        queries: Vec<FederatedMultiSearchQuery>,
        federation: Federation,
    ) -> Result<FederatedSearchResult> {
        let start_time = std::time::Instant::now();

        // Collect all hits from every query, weighted by federation_options.
        let mut all_hits = Vec::new();
        let mut facets_by_index: std::collections::BTreeMap<String, ComputedFacets> =
            std::collections::BTreeMap::new();
        let mut total_semantic_hits: u32 = 0;

        for fmq in &queries {
            let index = self.get_index(&fmq.index_uid)?;
            let result = index.search(&fmq.query)?;

            let weight = fmq
                .federation_options
                .as_ref()
                .map(|o| o.weight)
                .unwrap_or(1.0);

            // Scale ranking scores by weight and collect hits.
            for mut hit in result.hits {
                if let Some(score) = hit.ranking_score {
                    hit.ranking_score = Some(score * weight);
                }
                all_hits.push(hit);
            }

            // Aggregate per-index facets when present.
            if result.facet_distribution.is_some() || result.facet_stats.is_some() {
                facets_by_index.insert(
                    fmq.index_uid.clone(),
                    ComputedFacets {
                        distribution: result.facet_distribution.unwrap_or_default(),
                        stats: result.facet_stats.unwrap_or_default(),
                    },
                );
            }

            if let Some(shc) = result.semantic_hit_count {
                total_semantic_hits += shc;
            }
        }

        // Sort hits by ranking score (descending). Hits without a score sort last.
        all_hits.sort_by(|a, b| {
            let sa = a.ranking_score.unwrap_or(f64::NEG_INFINITY);
            let sb = b.ranking_score.unwrap_or(f64::NEG_INFINITY);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_hits = all_hits.len();

        // Apply federation-level pagination.
        let (hits_info, hits) = if let Some(page) = federation.page {
            let hpp = federation.hits_per_page.unwrap_or(20);
            let skip = (page.saturating_sub(1)) * hpp;
            let total_pages = if hpp > 0 {
                (total_hits + hpp - 1) / hpp
            } else {
                0
            };
            let paginated: Vec<_> = all_hits.into_iter().skip(skip).take(hpp).collect();
            (
                HitsInfo::Pagination {
                    hits_per_page: hpp,
                    page,
                    total_pages,
                    total_hits,
                },
                paginated,
            )
        } else {
            let paginated: Vec<_> = all_hits
                .into_iter()
                .skip(federation.offset)
                .take(federation.limit)
                .collect();
            (
                HitsInfo::OffsetLimit {
                    limit: federation.limit,
                    offset: federation.offset,
                    estimated_total_hits: total_hits,
                },
                paginated,
            )
        };

        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(FederatedSearchResult {
            hits,
            processing_time_ms,
            hits_info,
            facet_distribution: None,
            facet_stats: None,
            facets_by_index,
            semantic_hit_count: if total_semantic_hits > 0 {
                Some(total_semantic_hits)
            } else {
                None
            },
        })
    }
}

fn is_valid_uid(uid: &str) -> bool {
    uid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut size = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                size += dir_size(&entry.path())?;
            } else {
                size += metadata.len();
            }
        }
    }
    Ok(size)
}
