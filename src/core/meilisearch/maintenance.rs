use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;

use crate::core::error::{Error, Result};
use crate::core::now_iso8601;

use super::{write_documents_to_json, save_index_metadata, DumpInfo, Meilisearch};

impl Meilisearch {
    /// Create a database dump at the specified directory.
    #[instrument(skip(self))]
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
            write_documents_to_json(&index, &index_dump_dir.join("documents.json"))?;
        }

        Ok(DumpInfo {
            uid,
            path: dump_path.to_string_lossy().to_string(),
        })
    }

    /// Export selected indexes to a filesystem directory.
    ///
    /// - `export_path` -- destination directory (created if absent)
    /// - `indexes` -- `None` exports all; `Some(map)` filters to those UIDs.
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

            // Export documents in batches
            write_documents_to_json(&index, &index_dir.join("documents.json"))?;
        }

        Ok(())
    }

    /// Create a snapshot of the database at the specified directory.
    ///
    /// Uses LMDB's built-in copy API on the already-open index environments to
    /// produce a consistent snapshot without risking corruption from copying live
    /// data files or opening duplicate LMDB environments.
    #[instrument(skip(self))]
    pub fn create_snapshot(&self, snapshot_dir: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(snapshot_dir)?;

        let index_uids = self.list_indexes()?;
        if index_uids.is_empty() {
            return Ok(());
        }

        let snapshot_indexes = snapshot_dir.join("indexes");
        std::fs::create_dir_all(&snapshot_indexes)?;

        for uid in &index_uids {
            let index = self.get_index(uid)?;
            let dest = snapshot_indexes.join(uid);
            std::fs::create_dir_all(&dest)?;

            // Use the already-open milli::Index to copy the LMDB env, avoiding
            // the danger of opening a second LMDB env on the same data directory.
            let snapshot_file = dest.join("data.mdb");
            let mut file = std::fs::File::create(&snapshot_file)?;
            index
                .inner
                .copy_to_file(&mut file, milli::heed::CompactionOption::Disabled)
                .map_err(Error::Milli)?;
        }

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
            let mut lock = self.indexes.write().unwrap_or_else(|e| {
                tracing::warn!(uid, "indexes RwLock poisoned in compact_index, recovering");
                e.into_inner()
            });
            let mut indexes = (**lock).clone();

            if let Some(index) = indexes.remove(uid) {
                // SAFETY: Arc::strong_count is generally unreliable for synchronization
                // because other threads can clone/drop Arcs concurrently. However, this
                // usage is sound because:
                //  1. We hold the WRITE lock on `indexes`, so no thread can enter
                //     `get_index` (which requires at least a READ lock) to obtain a new
                //     clone of this Arc.
                //  2. The only way to obtain an Arc<Index> is through `get_index` or
                //     `create_index`, both of which acquire the `indexes` lock.
                //  3. Therefore the strong count cannot increase while we hold the write
                //     lock. Existing clones may still be live (count > 2), and that is
                //     exactly what we are detecting here.
                //  4. Count == 2: one in the original map (inside `lock`), one in `index`.
                if Arc::strong_count(&index) > 2 {
                    return Err(Error::IndexInUse(uid.to_string()));
                }

                drop(index); // drop our reference

                // Update the lock to the new map (without the index)
                *lock = Arc::new(indexes);
            }
        }

        // Perform copy-compact: open env, copy compact to temp, replace original
        let tmp_path = self.options.db_path.join("indexes").join(format!("_compact_tmp_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_path)?;

        {
            let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
            options.map_size(self.options.max_index_size);

            // SAFETY: The LMDB environment is opened on an existing index directory
            // solely for copy-compact. The path is constructed from our own `db_path`
            // + validated index UID. The env is scoped to this block and dropped
            // before we replace the data file.
            #[allow(unsafe_code)]
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
        if let Err(e) = std::fs::remove_dir_all(&tmp_path) {
            tracing::warn!(uid, error = %e, "failed to clean up compact temp directory");
        }

        Ok(())
    }

    /// Atomically swap the contents of pairs of indexes.
    #[instrument(skip(self, swaps))]
    pub fn swap_indexes(&self, swaps: &[(&str, &str)]) -> Result<()> {
        // Hold write lock for the full swap sequence to prevent TOCTOU race.
        let mut lock = self.indexes.write().unwrap_or_else(|e| {
            tracing::warn!("indexes RwLock poisoned in swap_indexes, recovering");
            e.into_inner()
        });

        // Validate all indexes exist under the write lock
        for (a, b) in swaps {
            if !self.index_exists(a) {
                return Err(Error::IndexNotFound(a.to_string()));
            }
            if !self.index_exists(b) {
                return Err(Error::IndexNotFound(b.to_string()));
            }
        }

        let mut indexes = (**lock).clone();

        for (a, b) in swaps {
            let index_path_a = self.options.db_path.join("indexes").join(a);
            let index_path_b = self.options.db_path.join("indexes").join(b);
            let tmp_path = self.options.db_path.join("indexes").join(format!("_swap_tmp_{}", Uuid::new_v4()));

            // Perform filesystem renames FIRST, before evicting from cache.
            // On Linux, renaming directories with open LMDB envs is safe because
            // mmap tracks by inode, not by path. If a rename fails the cache
            // remains consistent with the (unchanged) disk layout.
            std::fs::rename(&index_path_a, &tmp_path)?;
            if let Err(e) = std::fs::rename(&index_path_b, &index_path_a) {
                if let Err(re) = std::fs::rename(&tmp_path, &index_path_a) {
                    tracing::error!(index = *a, error = %re, "swap recovery failed: could not restore index from tmp");
                }
                return Err(e.into());
            }
            if let Err(e) = std::fs::rename(&tmp_path, &index_path_b) {
                if let Err(re) = std::fs::rename(&index_path_a, &index_path_b) {
                    tracing::error!(index = *b, error = %re, "swap recovery failed: could not restore index");
                }
                if let Err(re) = std::fs::rename(&tmp_path, &index_path_a) {
                    tracing::error!(index = *a, error = %re, "swap recovery failed: could not restore index from tmp");
                }
                return Err(e.into());
            }

            // Renames succeeded -- now evict from cache so the indexes are
            // re-opened from their new (swapped) paths on next access.
            let idx_a = indexes.remove(*a);
            let idx_b = indexes.remove(*b);
            drop(idx_a);
            drop(idx_b);

            // Swap metadata entries and bump updated_at
            let now = now_iso8601();
            let mut meta_cache = self.index_metadata.write().unwrap_or_else(|e| {
                tracing::warn!("index_metadata RwLock poisoned in swap_indexes, recovering");
                e.into_inner()
            });
            let meta_a = meta_cache.remove(*a);
            let meta_b = meta_cache.remove(*b);
            if let Some(mut m) = meta_b {
                m.updated_at = now.clone();
                if let Err(e) = save_index_metadata(&index_path_a, &m) {
                    tracing::warn!(index = *a, error = %e, "failed to persist swapped metadata");
                }
                meta_cache.insert(a.to_string(), m);
            }
            if let Some(mut m) = meta_a {
                m.updated_at = now;
                if let Err(e) = save_index_metadata(&index_path_b, &m) {
                    tracing::warn!(index = *b, error = %e, "failed to persist swapped metadata");
                }
                meta_cache.insert(b.to_string(), m);
            }
        }

        *lock = Arc::new(indexes);

        // Indexes will be re-loaded on next access via get_index()
        Ok(())
    }
}
