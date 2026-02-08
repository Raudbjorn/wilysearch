//! HTTP-less Meilisearch engine implementation.
//!
//! Wraps `crate::core::Meilisearch` and implements all traits from
//! `crate::traits` by delegating directly to the milli search engine.
//! Operations execute synchronously -- no task queue. Mutation methods
//! return a synthetic `TaskInfo` with status `Succeeded`.

mod conversion;
mod documents;
mod indexes;
mod search;
mod settings;
mod stubs;
mod system;

use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::now_iso8601;
use crate::traits::Result;
use crate::types::*;

/// Embedded Meilisearch engine backed by milli/LMDB.
///
/// All operations execute immediately. Mutations return a synthetic
/// [`TaskInfo`] with `status: Succeeded` since there is no task queue.
///
/// # Drop behavior
///
/// When `Engine` is dropped, all cached `milli::Index` handles are released.
/// Each `milli::Index` wraps a `heed::Env` which flushes pending writes and
/// closes the LMDB environment in its own `Drop` implementation. No explicit
/// `Drop` impl is needed on `Engine` itself.
pub struct Engine {
    inner: crate::core::Meilisearch,
    /// In-memory task counter; resets to 0 on restart. Task UIDs are not
    /// persisted and may collide across engine restarts. Consumers that need
    /// stable task references should use their own persistent counter.
    task_counter: AtomicU64,
    task_counter_path: std::path::PathBuf,
    dump_dir: std::path::PathBuf,
    snapshot_dir: std::path::PathBuf,
    /// Serializes dump/snapshot operations so concurrent callers do not
    /// corrupt the output archive. Wire into `create_dump`/`create_snapshot`
    /// once those methods perform real I/O.
    dump_lock: std::sync::Mutex<()>,
    /// RAG/preprocessing configuration loaded at startup.
    ///
    /// Not yet wired into the search path. See TODOs below.
    // TODO: apply search defaults (limit, matching strategy) from rag_config in convert_search_request
    // TODO: initialize preprocessing pipeline (SymSpell/synonyms) from rag_config at Engine construction
    // TODO: build RAG pipeline (embedder + retriever + reranker) on demand for retrieval search requests
    #[allow(dead_code)]
    rag_config: crate::config::RagConfig,
}

impl Engine {
    /// Create a new embedded engine with the given options.
    pub fn new(options: crate::core::MeilisearchOptions) -> Result<Self> {
        let dump_dir = options.db_path.join("dumps");
        let snapshot_dir = options.db_path.join("snapshots");
        let task_counter_path = options.db_path.join("task_counter");

        let start_uid = if task_counter_path.exists() {
            std::fs::read_to_string(&task_counter_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        let inner = crate::core::Meilisearch::new(options)?;
        Ok(Self {
            inner,
            task_counter: AtomicU64::new(start_uid),
            task_counter_path,
            dump_dir,
            snapshot_dir,
            dump_lock: std::sync::Mutex::new(()),
            rag_config: crate::config::RagConfig::default(),
        })
    }

    /// Create a new engine with default options.
    pub fn default_engine() -> Result<Self> {
        Self::new(crate::core::MeilisearchOptions::default())
    }

    /// Create a new engine from a [`WilysearchConfig`](crate::config::WilysearchConfig).
    ///
    /// This applies all configuration sections:
    /// - **engine** -- LMDB database path and mmap sizes
    /// - **experimental** -- runtime feature flags
    /// - **vector_store** -- SurrealDB vector store (requires `surrealdb` feature)
    /// - **search_defaults**, **preprocessing**, **rag** -- stored for later use
    pub fn with_config(config: crate::config::WilysearchConfig) -> Result<Self> {
        let options: crate::core::MeilisearchOptions = config.engine.into();
        let dump_dir = options.db_path.join("dumps");
        let snapshot_dir = options.db_path.join("snapshots");
        let task_counter_path = options.db_path.join("task_counter");

        let start_uid = if task_counter_path.exists() {
            std::fs::read_to_string(&task_counter_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        #[allow(unused_mut)]
        let mut inner = crate::core::Meilisearch::new(options)?;

        // Attach SurrealDB vector store if configured (feature-gated).
        #[cfg(feature = "surrealdb")]
        {
            if let Some(vs_config) = config.vector_store {
                let surreal_config: crate::core::vector::surrealdb::SurrealDbVectorStoreConfig =
                    vs_config.into();
                // Use a current_thread runtime for lower overhead as requested in review
                let rt = std::sync::Arc::new(
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| crate::core::error::Error::Internal(e.to_string()))?
                );
                let store = rt
                    .block_on(
                        crate::core::vector::surrealdb::SurrealDbVectorStore::with_runtime(
                            surreal_config,
                            rt.clone(),
                        ),
                    )
                    .map_err(|e| crate::core::error::Error::Internal(e.to_string()))?;
                inner = inner.with_vector_store(std::sync::Arc::new(store));
            }
        }

        // Apply experimental feature flags.
        let exp_features: crate::core::ExperimentalFeatures = config.experimental.into();
        inner.update_experimental_features(exp_features);

        Ok(Self {
            inner,
            task_counter: AtomicU64::new(start_uid),
            task_counter_path,
            dump_dir,
            snapshot_dir,
            dump_lock: std::sync::Mutex::new(()),
            rag_config: config.rag,
        })
    }

    /// Create a new engine from a TOML configuration file.
    ///
    /// Loads the file with [`WilysearchConfig::from_file`](crate::config::WilysearchConfig::from_file),
    /// applying environment variable overrides, then delegates to [`Engine::with_config`].
    pub fn from_config_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let config = crate::config::WilysearchConfig::from_file(path).map_err(|e| {
            crate::core::error::Error::Internal(e.to_string())
        })?;
        Self::with_config(config)
    }

    fn next_task(&self, task_type: &str, index_uid: Option<&str>) -> TaskInfo {
        let uid = self.task_counter.fetch_add(1, Ordering::Relaxed);
        // Non-atomic write: a crash mid-write could truncate this file, causing
        // counter reuse after restart. This is acceptable because task UIDs are
        // ephemeral in embedded mode (no persistent task queue) and collisions
        // only affect synthetic TaskInfo.task_uid values.
        if let Err(e) = std::fs::write(&self.task_counter_path, (uid + 1).to_string()) {
            tracing::warn!(error = %e, "failed to persist task counter");
        }
        TaskInfo {
            task_uid: uid,
            index_uid: index_uid.map(String::from),
            status: TaskStatus::Succeeded,
            r#type: task_type.to_string(),
            enqueued_at: now_iso8601(),
        }
    }

    fn resolve_index(&self, uid: &str) -> Result<std::sync::Arc<crate::core::Index>> {
        Ok(self.inner.get_index(uid)?)
    }

    fn mutation_task(&self, index_uid: &str, task_type: &str) -> Result<TaskInfo> {
        self.inner.touch_index_updated(index_uid)?;
        Ok(self.next_task(task_type, Some(index_uid)))
    }
}

// ─── Type conversion helpers ─────────────────────────────────────────────────

/// Saturating cast from `usize` to `u32`.
/// Returns `u32::MAX` when the value exceeds `u32::MAX` instead of silently
/// truncating via `as u32`.
pub(super) fn saturating_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Infallible cast from `usize` to `u64`.
/// On platforms where `usize` is 64-bit this is a no-op; on 32-bit platforms
/// the value always fits. Included for consistency with `saturating_u32`.
pub(super) fn usize_to_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// Saturating cast from `u128` to `u64`.
/// Returns `u64::MAX` when the value exceeds `u64::MAX` (e.g. processing
/// times that overflow 64 bits -- astronomically unlikely but handled safely).
pub(super) fn saturating_u128_to_u64(v: u128) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

// Static assertions: Engine must be Send + Sync for safe sharing across threads.
const _: () = {
    #[allow(dead_code)]
    fn assert_send_sync<T: Send + Sync>() {}
    #[allow(dead_code)]
    fn assert_engine() { assert_send_sync::<Engine>(); }
};
