//! `System` and `ExperimentalFeaturesApi` trait implementations for `Engine`.

use std::collections::HashMap;

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;

impl traits::System for Engine {
    fn global_stats(&self) -> Result<GlobalStats> {
        let lib_stats = self.inner.stats()?;
        Ok(GlobalStats {
            database_size: lib_stats.database_size,
            used_database_size: None,
            last_update: lib_stats.last_update,
            indexes: lib_stats
                .indexes
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        IndexStats {
                            number_of_documents: v.number_of_documents,
                            is_indexing: v.is_indexing,
                            field_distribution: v.field_distribution.into_iter().collect(),
                        },
                    )
                })
                .collect(),
        })
    }

    fn index_stats(&self, index_uid: &str) -> Result<IndexStats> {
        let lib_stats = self.inner.index_stats(index_uid)?;
        Ok(IndexStats {
            number_of_documents: lib_stats.number_of_documents,
            is_indexing: lib_stats.is_indexing,
            field_distribution: lib_stats.field_distribution.into_iter().collect(),
        })
    }

    fn version(&self) -> Result<Version> {
        let v = self.inner.version();
        Ok(Version {
            commit_sha: v.commit_sha,
            commit_date: v.commit_date,
            pkg_version: v.pkg_version,
        })
    }

    fn health(&self) -> Result<Health> {
        let h = self.inner.health();
        Ok(Health { status: h.status })
    }

    fn create_dump(&self) -> Result<TaskInfo> {
        let _guard = self.dump_lock.lock().unwrap_or_else(|e| {
            tracing::warn!("dump_lock Mutex poisoned in create_dump, recovering");
            e.into_inner()
        });
        std::fs::create_dir_all(&self.dump_dir)?;
        self.inner.create_dump(&self.dump_dir)?;
        Ok(self.next_task("dumpCreation", None))
    }

    fn create_snapshot(&self) -> Result<TaskInfo> {
        let _guard = self.dump_lock.lock().unwrap_or_else(|e| {
            tracing::warn!("dump_lock Mutex poisoned in create_snapshot, recovering");
            e.into_inner()
        });
        std::fs::create_dir_all(&self.snapshot_dir)?;
        self.inner.create_snapshot(&self.snapshot_dir)?;
        Ok(self.next_task("snapshotCreation", None))
    }

    fn export(&self, request: &ExportRequest) -> Result<TaskInfo> {
        let raw_path = std::path::Path::new(&request.url);

        // Canonicalize the export path to prevent directory traversal.
        // Create the directory first so canonicalize has a real path to resolve.
        std::fs::create_dir_all(raw_path)?;
        let export_path = raw_path.canonicalize().map_err(|e| {
            crate::core::error::Error::Internal(format!(
                "Failed to canonicalize export path '{}': {e}",
                request.url
            ))
        })?;

        // Belt-and-suspenders: canonicalize() resolves symlinks and removes
        // `..` components, so this check should never trigger. Kept as
        // defense-in-depth against hypothetical platform edge cases.
        if export_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(crate::core::error::Error::Internal(
                "Export path must not contain '..' components".to_string(),
            ).into());
        }

        let index_settings: Option<HashMap<String, bool>> = request.indexes.as_ref().map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.override_settings.unwrap_or(true)))
                .collect()
        });

        self.inner.export(&export_path, index_settings.as_ref())?;
        Ok(self.next_task("export", None))
    }
}

// ─── Experimental features ───────────────────────────────────────────────────

impl traits::ExperimentalFeaturesApi for Engine {
    fn get_experimental_features(&self) -> Result<ExperimentalFeatures> {
        let lib_features = self.inner.get_experimental_features();
        Ok(ExperimentalFeatures {
            features: serde_json::from_value(serde_json::to_value(&lib_features)?)?,
        })
    }

    fn update_experimental_features(
        &self,
        features: &ExperimentalFeatures,
    ) -> Result<ExperimentalFeatures> {
        let lib_features: crate::core::meilisearch::ExperimentalFeatures =
            serde_json::from_value(serde_json::to_value(&features.features)?)?;
        let updated = self.inner.update_experimental_features(lib_features);
        Ok(ExperimentalFeatures {
            features: serde_json::from_value(serde_json::to_value(&updated)?)?,
        })
    }
}
