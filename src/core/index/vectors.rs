use serde_json::Value;

use crate::core::error::{Error, Result};

use super::{Index, pk_value_to_string};

impl Index {
    /// Parse all vectors from a `_vectors` JSON value.
    ///
    /// Supports three formats per embedder:
    /// - `{"embedder": [f32, ...]}` -- single vector
    /// - `{"embedder": [[f32, ...], ...]}` -- multi-vector
    /// - `{"embedder": {"embeddings": [...], "regenerate": bool}}` -- structured
    ///
    /// Returns all vectors from all embedders flattened into one list.
    pub(crate) fn parse_vectors_value(vectors_val: &Value) -> Result<Vec<Vec<f32>>> {
        let obj = match vectors_val.as_object() {
            Some(o) => o,
            None => return Ok(Vec::new()),
        };

        let mut all_vectors = Vec::new();

        for (_embedder_name, val) in obj {
            match val {
                // Single vector: [f32, ...]
                Value::Array(arr) if arr.first().map_or(false, |v| v.is_number()) => {
                    let vec: Vec<f32> = arr
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect();
                    if !vec.is_empty() {
                        all_vectors.push(vec);
                    }
                }
                // Multi-vector: [[f32, ...], ...]
                Value::Array(arr) if arr.first().map_or(false, |v| v.is_array()) => {
                    for inner in arr {
                        if let Value::Array(nums) = inner {
                            let vec: Vec<f32> = nums
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect();
                            if !vec.is_empty() {
                                all_vectors.push(vec);
                            }
                        }
                    }
                }
                // Structured: {"embeddings": [...], "regenerate": bool}
                Value::Object(inner) => {
                    if let Some(embeddings) = inner.get("embeddings") {
                        if let Value::Array(emb_arr) = embeddings {
                            for item in emb_arr {
                                match item {
                                    // [[f32, ...], ...]
                                    Value::Array(nums) if nums.first().map_or(false, |v| v.is_number()) => {
                                        let vec: Vec<f32> = nums
                                            .iter()
                                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                                            .collect();
                                        if !vec.is_empty() {
                                            all_vectors.push(vec);
                                        }
                                    }
                                    // [[[f32, ...], ...]]
                                    Value::Array(inner_arr) if inner_arr.first().map_or(false, |v| v.is_array()) => {
                                        for nested in inner_arr {
                                            if let Value::Array(nums) = nested {
                                                let vec: Vec<f32> = nums
                                                    .iter()
                                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                                    .collect();
                                                if !vec.is_empty() {
                                                    all_vectors.push(vec);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(all_vectors)
    }

    /// Scan input documents for `_vectors` fields and pair each with its external ID string.
    ///
    /// Returns an empty vec if no primary key is known or no vectors are found.
    pub(crate) fn extract_pending_vectors(
        documents: &[Value],
        pk_name: Option<&str>,
    ) -> Result<Vec<(String, Vec<Vec<f32>>)>> {
        let pk_name = match pk_name {
            Some(name) => name,
            None => return Ok(Vec::new()),
        };

        let mut pending = Vec::new();

        for doc in documents {
            let obj = match doc.as_object() {
                Some(o) => o,
                None => continue,
            };

            let vectors_val = match obj.get("_vectors") {
                Some(v) => v,
                None => continue,
            };

            let pk_val = match obj.get(pk_name) {
                Some(v) => v,
                None => continue,
            };

            let vectors = Self::parse_vectors_value(vectors_val)?;
            if !vectors.is_empty() {
                pending.push((pk_value_to_string(pk_val), vectors));
            }
        }

        Ok(pending)
    }

    /// Sync pending vectors to the external VectorStore AFTER the LMDB write
    /// transaction has been committed.
    ///
    /// Opens a fresh read transaction to resolve external IDs to internal IDs.
    /// This ensures LMDB is the source of truth: documents are persisted first,
    /// and vector sync is a best-effort follow-up. If vector sync fails, the
    /// documents are still safely stored and vectors can be re-synced later.
    ///
    /// No-op if no vector store is configured or the pending list is empty.
    pub(crate) fn sync_pending_vectors_post_commit(
        &self,
        pending: Vec<(String, Vec<Vec<f32>>)>,
    ) -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let store = match &self.vector_store {
            Some(s) => s,
            None => return Ok(()),
        };

        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let external_ids = self.inner.external_documents_ids();
        let mut pairs: Vec<(u32, Vec<Vec<f32>>)> = Vec::with_capacity(pending.len());

        for (ext_id, vectors) in pending {
            if let Some(internal_id) = external_ids.get(&rtxn, &ext_id).map_err(Error::Heed)? {
                pairs.push((internal_id, vectors));
            }
        }

        if !pairs.is_empty() {
            store
                .add_documents(&pairs)
                .map_err(|e| Error::VectorStore(e.to_string()))?;
        }

        Ok(())
    }
}
