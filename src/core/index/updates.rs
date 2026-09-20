use milli::Filter;
use milli::documents::PrimaryKey;
use milli::progress::EmbedderStats;
use milli::update::IndexerConfig;
use serde_json::Value;
use std::sync::Arc;
use tracing::instrument;

use crate::core::error::{Error, Result};

use super::Index;

impl Index {
    /// Add or replace documents in the index.
    ///
    /// Each document must be a JSON object. If a document with the same primary
    /// key already exists it is entirely replaced.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// let docs = vec![
    ///     serde_json::json!({"id": 1, "title": "Interstellar"}),
    ///     serde_json::json!({"id": 2, "title": "Inception"}),
    /// ];
    /// index.add_documents(docs, None)?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if a document is not a JSON object or if the indexing
    /// operation fails.
    #[instrument(skip(self, documents))]
    pub fn add_documents(&self, documents: Vec<Value>, primary_key: Option<&str>) -> Result<()> {
        self.index_documents(documents, primary_key, false, false)
    }

    /// Update (partially merge) documents into the index.
    ///
    /// Unlike `add_documents`, this uses `IndexDocumentsMethod::UpdateDocuments`
    /// which merges the provided fields into existing documents rather than
    /// replacing them entirely. Only the fields present in the new document
    /// are overwritten; other existing fields are preserved.
    pub fn update_documents(&self, documents: Vec<Value>, primary_key: Option<&str>) -> Result<()> {
        self.index_documents(documents, primary_key, true, false)
    }

    pub fn index_documents(
        &self,
        documents: Vec<Value>,
        primary_key: Option<&str>,
        merge: bool,
        skip_creation: bool,
    ) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(|e| Error::Heed(e))?;

        // If a primary key is provided and the index doesn't have one yet, set it
        if let Some(pk) = primary_key {
            let existing_pk = self.inner.primary_key(&wtxn).map_err(Error::Heed)?;
            if existing_pk.is_none() {
                let indexer_config = IndexerConfig::default();
                let mut settings =
                    milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
                settings.set_primary_key(pk.to_string());
                let ip_policy = self.ip_policy.clone();
                let embedder_stats = Arc::new(EmbedderStats::default());
                let progress = milli::progress::Progress::quiet();
                settings
                    .execute(
                        &milli::MustStopProcessing::default(),
                        &progress,
                        &ip_policy,
                        embedder_stats,
                    )
                    .map_err(Error::Milli)?;
            }
        }

        // Extract pending vectors before the batch loop consumes documents.
        // Use the provided PK or read the one already set on the index.
        let pk_name = match primary_key {
            Some(pk) => Some(pk.to_string()),
            None => self
                .inner
                .primary_key(&wtxn)
                .map_err(Error::Heed)?
                .map(|s| s.to_string()),
        };
        let pending_vectors = Self::extract_pending_vectors(&documents, pk_name.as_deref())?;

        let indexer_config = IndexerConfig::default();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let ip_policy = self.ip_policy.clone();

        use milli::update::{InnerIndexSettings, MissingDocumentPolicy, new::indexer};
        let mut payload = Vec::new();
        for doc in &documents {
            if !doc.is_object() {
                return Err(Error::Internal("Document must be a JSON object".into()));
            }
            serde_json::to_writer(&mut payload, doc)?;
            payload.push(b'\n');
        }
        {
            let rtxn = self.inner.read_txn()?;
            let fields = self.inner.fields_ids_map(&rtxn)?;
            let mut new_fields = fields.clone();
            let embedders = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)?
                .runtime_embedders;
            let mut ops = indexer::IndexOperations::new();
            let on_missing_document = if skip_creation {
                MissingDocumentPolicy::Skip
            } else {
                MissingDocumentPolicy::Create
            };
            ops.push_raw_operation(if merge {
                indexer::Payload::Update {
                    payload: &payload,
                    on_missing_document,
                }
            } else {
                indexer::Payload::Replace {
                    payload: &payload,
                    on_missing_document,
                }
            });
            let arena = bumpalo::Bump::new();
            let stop = milli::MustStopProcessing::default();
            let progress = milli::progress::Progress::quiet();
            let (changes, stats, pk) = ops.into_changes(
                &arena,
                &self.inner,
                &rtxn,
                primary_key,
                &mut new_fields,
                &stop,
                progress.clone(),
                None,
            )?;
            if let Some(error) = stats.into_iter().find_map(|s| s.error) {
                return Err(Error::Milli(error.into()));
            }
            let pool = milli::ThreadPoolNoAbortBuilder::new()
                .build()
                .map_err(|e| Error::Internal(e.to_string()))?;
            indexer_config
                .thread_pool
                .install(|| {
                    indexer::index(
                        &mut wtxn,
                        &self.inner,
                        &pool,
                        indexer_config.grenad_parameters(),
                        &fields,
                        new_fields,
                        pk,
                        &changes,
                        embedders,
                        &stop,
                        &progress,
                        &ip_policy,
                        &embedder_stats,
                    )
                })
                .map_err(|e| Error::Internal(e.to_string()))??;
        }

        wtxn.commit().map_err(|e| Error::Heed(e))?;

        // Sync extracted vectors to the external VectorStore AFTER LMDB commit
        // so that LMDB remains the source of truth. If vector sync fails the
        // documents are still safely persisted and vectors can be re-synced.
        self.sync_pending_vectors_post_commit(pending_vectors)?;

        Ok(())
    }

    /// Delete a single document by its external ID.
    ///
    /// Returns `true` if the document was deleted, `false` if it didn't exist.
    pub fn delete_document(&self, id: &str) -> Result<bool> {
        self.delete_documents(vec![id.to_string()])
            .map(|count| count > 0)
    }

    /// Delete multiple documents by their external IDs.
    ///
    /// Returns the number of documents that were actually deleted.
    /// Documents that don't exist are silently ignored.
    pub fn delete_documents(&self, ids: Vec<String>) -> Result<u64> {
        use bumpalo::Bump;
        use milli::update::InnerIndexSettings;
        use milli::update::new::indexer;

        if ids.is_empty() {
            return Ok(0);
        }

        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;

        // Pre-deletion reads use a scoped rtxn. We extract owned data
        // (existing_count, vector_ids) so the rtxn can be dropped immediately.
        let (existing_count, vector_ids_to_remove) = {
            let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
            let external_ids = self.inner.external_documents_ids();
            let mut count = 0u64;
            let mut to_remove: Vec<u32> = Vec::new();
            for id in &ids {
                if let Some(internal_id) = external_ids.get(&rtxn, id).map_err(Error::Heed)? {
                    count += 1;
                    if self.vector_store.is_some() {
                        to_remove.push(internal_id);
                    }
                }
            }
            (count, to_remove)
        };

        if existing_count == 0 {
            return Ok(0);
        }

        // Scope the indexer rtxn: document_changes borrows rtxn (via into_changes),
        // so both must live through the indexer::index call. The block ensures the
        // rtxn is dropped before wtxn.commit(), allowing LMDB page reclamation.
        {
            let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

            let indexer_config = IndexerConfig::default();
            let pool = &indexer_config.thread_pool;

            let db_fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
            let mut new_fields_ids_map = db_fields_ids_map.clone();

            let ip_policy = self.ip_policy.clone();
            let embedders = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
                .map_err(Error::Milli)?
                .runtime_embedders;

            let mut indexer_ops = indexer::IndexOperations::new();
            let id_refs: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
            indexer_ops.delete_documents_by_external_ids(&id_refs);

            let indexer_alloc = Bump::new();
            let (document_changes, operation_stats, primary_key) = indexer_ops
                .into_changes(
                    &indexer_alloc,
                    &self.inner,
                    &rtxn,
                    None,
                    &mut new_fields_ids_map,
                    &milli::MustStopProcessing::default(),
                    milli::progress::Progress::quiet(),
                    None,
                )
                .map_err(Error::Milli)?;

            if let Some(error) = operation_stats.into_iter().find_map(|stat| stat.error) {
                return Err(Error::Milli(error.into()));
            }

            let grenad_params = indexer_config.grenad_parameters();
            let indexing_pool = milli::ThreadPoolNoAbortBuilder::new()
                .build()
                .map_err(|e| {
                    Error::Internal(format!("failed to build indexing thread pool: {e}"))
                })?;

            pool.install(|| {
                indexer::index(
                    &mut wtxn,
                    &self.inner,
                    &indexing_pool,
                    grenad_params,
                    &db_fields_ids_map,
                    new_fields_ids_map,
                    primary_key,
                    &document_changes,
                    embedders,
                    &milli::MustStopProcessing::default(),
                    &milli::progress::Progress::quiet(),
                    &ip_policy,
                    &Default::default(),
                )
            })
            .map_err(|e| Error::Internal(e.to_string()))?
            .map_err(Error::Milli)?;
        } // rtxn + document_changes dropped here, before commit

        wtxn.commit().map_err(Error::Heed)?;

        // Remove vectors from the external VectorStore AFTER LMDB commit
        // so that LMDB remains the source of truth. If vector sync fails the
        // documents are still safely deleted and vectors can be re-synced.
        if let Some(store) = &self.vector_store {
            if !vector_ids_to_remove.is_empty() {
                store
                    .remove_documents(&vector_ids_to_remove)
                    .map_err(|e| Error::VectorStore(e.to_string()))?;
            }
        }

        Ok(existing_count)
    }

    /// Delete documents matching a filter expression.
    ///
    /// The filter uses the same syntax as search filters (e.g., "genre = 'horror'" or "year > 2000").
    /// Returns the number of documents deleted.
    ///
    /// Note: The field used in the filter must be configured as filterable in the index settings.
    pub fn delete_by_filter(&self, filter: &str) -> Result<u64> {
        let filter =
            Filter::from_str(filter)?.ok_or_else(|| Error::InvalidFilter("Empty filter".into()))?;
        self.delete_by_index_filter(super::local_filter(filter)?)
    }

    pub(crate) fn delete_by_index_filter(&self, filter: milli::IndexFilter) -> Result<u64> {
        use bumpalo::Bump;
        use milli::update::InnerIndexSettings;
        use milli::update::new::indexer::{self, DocumentDeletion};

        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;

        // Pre-deletion reads use a scoped rtxn. We extract owned data
        // (candidates, count, vector_ids) so the rtxn can be dropped immediately.
        let (candidates, count, vector_ids_to_remove) = {
            let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

            let candidates =
                filter.evaluate(&rtxn, &self.inner, &self.inner.fields_ids_map(&rtxn)?)?;
            let count = candidates.len();
            let vector_ids: Vec<u32> = if self.vector_store.is_some() {
                candidates.iter().collect()
            } else {
                Vec::new()
            };

            (candidates, count, vector_ids)
        };

        if count == 0 {
            return Ok(0);
        }

        // Scope the indexer rtxn: embedders borrows rtxn, and document_changes
        // borrows indexer_alloc + primary_key. The block ensures the rtxn is
        // dropped before wtxn.commit(), allowing LMDB page reclamation.
        {
            let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

            let indexer_config = IndexerConfig::default();
            let pool = &indexer_config.thread_pool;

            let db_fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
            let new_fields_ids_map = db_fields_ids_map.clone();

            let ip_policy = self.ip_policy.clone();
            let embedders = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
                .map_err(Error::Milli)?
                .runtime_embedders;

            let primary_key_str = self
                .inner
                .primary_key(&rtxn)
                .map_err(Error::Heed)?
                .ok_or_else(|| Error::Internal("Index has no primary key configured".to_string()))?
                .to_string();
            let primary_key =
                PrimaryKey::new(&primary_key_str, &db_fields_ids_map).ok_or_else(|| {
                    Error::Internal(format!(
                        "Primary key '{}' not found in fields map",
                        primary_key_str
                    ))
                })?;

            let mut deletion = DocumentDeletion::new();
            deletion.delete_documents_by_docids(candidates);

            let indexer_alloc = Bump::new();
            let document_changes = deletion.into_changes(&indexer_alloc, primary_key);

            let grenad_params = indexer_config.grenad_parameters();
            let indexing_pool = milli::ThreadPoolNoAbortBuilder::new()
                .build()
                .map_err(|e| {
                    Error::Internal(format!("failed to build indexing thread pool: {e}"))
                })?;

            pool.install(|| {
                indexer::index(
                    &mut wtxn,
                    &self.inner,
                    &indexing_pool,
                    grenad_params,
                    &db_fields_ids_map,
                    new_fields_ids_map,
                    None, // primary_key already set
                    &document_changes,
                    embedders,
                    &milli::MustStopProcessing::default(),
                    &milli::progress::Progress::quiet(),
                    &ip_policy,
                    &Default::default(),
                )
            })
            .map_err(|e| Error::Internal(e.to_string()))?
            .map_err(Error::Milli)?;
        } // rtxn + embedders + document_changes dropped here, before commit

        wtxn.commit().map_err(Error::Heed)?;

        // Remove vectors from the external VectorStore AFTER LMDB commit
        // so that LMDB remains the source of truth. If vector sync fails the
        // documents are still safely deleted and vectors can be re-synced.
        if let Some(store) = &self.vector_store {
            if !vector_ids_to_remove.is_empty() {
                store
                    .remove_documents(&vector_ids_to_remove)
                    .map_err(|e| Error::VectorStore(e.to_string()))?;
            }
        }

        Ok(count)
    }
}

impl Index {
    pub(crate) fn update_by_function(
        &self,
        filter: Option<milli::IndexFilter>,
        context: Option<milli::Object>,
        code: String,
    ) -> Result<()> {
        use milli::update::{InnerIndexSettings, new::indexer};
        let mut txn = self.inner.write_txn()?;
        let read = self.inner.read_txn()?;
        let fields = self.inner.fields_ids_map(&read)?;
        let candidates = match filter {
            Some(filter) => filter.evaluate(&read, &self.inner, &fields)?,
            None => self.inner.documents_ids(&read)?,
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let mut new_fields = fields.clone();
        let name = self
            .inner
            .primary_key(&read)?
            .ok_or_else(|| Error::Internal("Missing primary key".into()))?;
        let key = PrimaryKey::new_or_insert(name, &mut new_fields).map_err(milli::Error::from)?;
        let ids = candidates.clone();
        let changes =
            indexer::UpdateByFunction::new(candidates, context, code).into_changes(&key)?;
        let config = IndexerConfig::default();
        let pool = milli::ThreadPoolNoAbortBuilder::new()
            .build()
            .map_err(|e| Error::Internal(e.to_string()))?;
        let embedders = InnerIndexSettings::from_index(&self.inner, &read, &self.ip_policy, None)?
            .runtime_embedders;
        config
            .thread_pool
            .install(|| {
                indexer::index(
                    &mut txn,
                    &self.inner,
                    &pool,
                    config.grenad_parameters(),
                    &fields,
                    new_fields,
                    None,
                    &changes,
                    embedders,
                    &milli::MustStopProcessing::default(),
                    &milli::progress::Progress::quiet(),
                    &self.ip_policy,
                    &Default::default(),
                )
            })
            .map_err(|e| Error::Internal(e.to_string()))??;
        drop(read);
        txn.commit()?;
        if let Some(store) = &self.vector_store {
            store.remove_documents(&ids.iter().collect::<Vec<_>>())?;
            let txn = self.inner.read_txn()?;
            let fields = self.inner.fields_ids_map(&txn)?;
            let documents = ids
                .iter()
                .filter(|id| self.inner.document(&txn, *id).is_ok())
                .map(|id| self.make_document(&txn, &fields, id, None, true, false))
                .collect::<Result<Vec<_>>>()?;
            let pending = Self::extract_pending_vectors(&documents, self.inner.primary_key(&txn)?)?;
            self.sync_pending_vectors_post_commit(pending)?;
        }
        Ok(())
    }
}
