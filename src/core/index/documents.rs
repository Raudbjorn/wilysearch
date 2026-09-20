use milli::progress::EmbedderStats;
use milli::update::IndexerConfig;
use serde_json::Value;
use std::sync::Arc;

use crate::core::error::{Error, Result};
use crate::core::search::GetDocumentsOptions;

use super::Index;

/// Result of a document retrieval operation with pagination info.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentsResult {
    /// The retrieved documents. Serializes as `"results"` to match the HTTP API.
    #[serde(rename = "results")]
    pub documents: Vec<Value>,
    /// Total number of documents in the index.
    pub total: u64,
    /// Offset used for this query.
    pub offset: usize,
    /// Limit used for this query.
    pub limit: usize,
}

impl Index {
    /// Returns the number of documents in the index.
    pub fn document_count(&self) -> Result<u64> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        self.inner.number_of_documents(&rtxn).map_err(Error::Milli)
    }

    /// Retrieve a single document by its external ID.
    ///
    /// Returns `None` if the document does not exist.
    pub fn get_document(&self, id: &str) -> Result<Option<Value>> {
        self.get_document_with_fields(id, None)
    }

    /// Retrieve a single document by its external ID, returning only the
    /// specified fields.
    ///
    /// When `fields` is `None`, all stored fields except vectors are returned (same as
    /// [`get_document`](Self::get_document)). When `Some`, only the listed
    /// field names are included in the result.
    ///
    /// Returns `Ok(None)` when no document with the given ID exists.
    pub fn get_document_with_fields(
        &self,
        id: &str,
        fields: Option<&[String]>,
    ) -> Result<Option<Value>> {
        Ok(self
            .get_documents_with_options(&GetDocumentsOptions {
                ids: Some(vec![id.to_owned()]),
                fields: fields.map(<[String]>::to_vec),
                limit: 1,
                ..Default::default()
            })?
            .documents
            .into_iter()
            .next())
    }

    /// Retrieve documents with pagination, including fields hidden from search.
    pub fn get_documents(&self, offset: usize, limit: usize) -> Result<DocumentsResult> {
        self.get_documents_with_options(&GetDocumentsOptions {
            offset,
            limit,
            ..Default::default()
        })
    }

    pub fn get_documents_with_options(
        &self,
        options: &GetDocumentsOptions,
    ) -> Result<DocumentsResult> {
        let filter = options
            .filter
            .as_ref()
            .map(super::parse_local_filter)
            .transpose()?
            .flatten();
        self.documents_with_filter(options, filter)
    }

    pub(crate) fn documents_with_filter(
        &self,
        options: &GetDocumentsOptions,
        filter: Option<milli::IndexFilter>,
    ) -> Result<DocumentsResult> {
        let txn = self.inner.read_txn()?;
        let fields = self.inner.fields_ids_map(&txn)?;
        let mut candidates = self.inner.documents_ids(&txn)?;
        if let Some(ids) = &options.ids {
            let mut selected = roaring::RoaringBitmap::new();
            for id in ids {
                if let Some(id) = self.inner.external_documents_ids().get(&txn, id)? {
                    selected.insert(id);
                }
            }
            candidates &= selected;
        }
        if let Some(filter) = filter {
            candidates &= filter.evaluate(&txn, &self.inner, &fields)?;
        }
        let total = candidates.len();
        let ids: Vec<u32> = if let Some(sort) = options.sort.as_ref().filter(|s| !s.is_empty()) {
            let progress = milli::progress::Progress::quiet();
            let mut search = self.inner.search(
                &txn,
                &self.uid,
                &fields,
                time::OffsetDateTime::now_utc(),
                &progress,
            );
            search
                .candidates(&candidates)
                .offset(options.offset)
                .limit(options.limit);
            search.sort_criteria(
                sort.iter()
                    .map(|s| s.parse().map_err(|e| Error::InvalidSort(format!("{e}"))))
                    .collect::<Result<_>>()?,
            );
            search.execute()?.documents_ids
        } else {
            candidates
                .iter()
                .skip(options.offset)
                .take(options.limit)
                .collect()
        };
        let mut documents = Vec::with_capacity(ids.len());
        for id in ids {
            documents.push(self.make_document(
                &txn,
                &fields,
                id,
                options.fields.as_deref(),
                options.retrieve_vectors,
                false,
            )?);
        }
        Ok(DocumentsResult {
            documents,
            total,
            offset: options.offset,
            limit: options.limit,
        })
    }

    /// Update the primary key of the index.
    ///
    /// The primary key can only be set on an empty index. If the index already
    /// contains documents, this returns `Error::PrimaryKeyAlreadyPresent`.
    pub fn update_primary_key(&self, primary_key: &str) -> Result<()> {
        // Use a single write txn for both the check and the update to avoid
        // a TOCTOU race (documents could be inserted between read and write).
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;

        let doc_count = self
            .inner
            .number_of_documents(&wtxn)
            .map_err(Error::Milli)?;
        if doc_count > 0 {
            return Err(Error::PrimaryKeyAlreadyPresent);
        }

        let indexer_config = IndexerConfig::default();
        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.set_primary_key(primary_key.to_string());

        let ip_policy = self.ip_policy.clone();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::quiet();

        milli_settings
            .execute(
                &milli::MustStopProcessing::default(),
                &progress,
                &ip_policy,
                embedder_stats,
            )
            .map_err(Error::Milli)?;

        wtxn.commit().map_err(Error::Heed)?;

        Ok(())
    }

    /// Delete all documents from the index.
    ///
    /// This clears all documents but preserves index settings (searchable attributes,
    /// filterable attributes, etc.).
    ///
    /// Returns the number of documents that were deleted.
    pub fn clear(&self) -> Result<u64> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;

        let clear_op = milli::update::ClearDocuments::new(&mut wtxn, &self.inner);
        let deleted_count = clear_op.execute().map_err(Error::Milli)?;

        // Clear the external VectorStore
        if let Some(store) = &self.vector_store {
            store
                .clear()
                .map_err(|e| Error::VectorStore(e.to_string()))?;
        }

        wtxn.commit().map_err(Error::Heed)?;

        Ok(deleted_count)
    }
}
