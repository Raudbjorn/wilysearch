use milli::progress::EmbedderStats;
use milli::update::IndexerConfig;
use milli::{AscDesc, FieldId, Filter};
use serde_json::Value;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;

use crate::core::error::{Error, Result};
use crate::core::search::GetDocumentsOptions;

use super::{parse_filter_to_string, Index};

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
    /// When `fields` is `None`, all displayed attributes are returned (same as
    /// [`get_document`](Self::get_document)). When `Some`, only the listed
    /// field names are included in the result.
    ///
    /// Returns `Ok(None)` when no document with the given ID exists.
    pub fn get_document_with_fields(
        &self,
        id: &str,
        fields: Option<&[String]>,
    ) -> Result<Option<Value>> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

        // Look up the internal document ID from the external ID
        let external_ids = self.inner.external_documents_ids();
        let internal_id = match external_ids.get(&rtxn, id).map_err(Error::Heed)? {
            Some(id) => id,
            None => return Ok(None),
        };

        // Get the document content
        let document = self.inner.document(&rtxn, internal_id).map_err(Error::Milli)?;

        // Convert to JSON, restricting to requested fields if specified
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        let display_ids: Vec<FieldId> = if let Some(requested) = fields {
            requested
                .iter()
                .filter_map(|name| fields_ids_map.id(name))
                .collect()
        } else {
            self.inner
                .displayed_fields_ids(&rtxn)
                .map_err(Error::Milli)?
                .map(|fields| fields.into_iter().collect::<Vec<_>>())
                .unwrap_or_else(|| fields_ids_map.ids().collect())
        };

        let json =
            milli::obkv_to_json(&display_ids, &fields_ids_map, document).map_err(Error::Milli)?;

        Ok(Some(Value::Object(json)))
    }

    /// Retrieve documents with pagination.
    ///
    /// - `offset`: Number of documents to skip (0-indexed).
    /// - `limit`: Maximum number of documents to return.
    pub fn get_documents(&self, offset: usize, limit: usize) -> Result<DocumentsResult> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

        let total = self.inner.number_of_documents(&rtxn).map_err(Error::Milli)?;

        // Get all document IDs and apply pagination
        let document_ids = self.inner.documents_ids(&rtxn).map_err(Error::Heed)?;
        let paginated_ids: Vec<u32> = document_ids.iter().skip(offset).take(limit).collect();

        if paginated_ids.is_empty() {
            return Ok(DocumentsResult { documents: Vec::new(), total, offset, limit });
        }

        // Fetch the documents
        let documents = self.inner.documents(&rtxn, paginated_ids).map_err(Error::Milli)?;

        // Convert to JSON
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
        let displayed_fields = self
            .inner
            .displayed_fields_ids(&rtxn)
            .map_err(Error::Milli)?
            .map(|fields| fields.into_iter().collect::<Vec<_>>())
            .unwrap_or_else(|| fields_ids_map.ids().collect());

        let mut docs = Vec::with_capacity(documents.len());
        for (_id, obkv) in documents {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            docs.push(Value::Object(json));
        }

        Ok(DocumentsResult { documents: docs, total, offset, limit })
    }

    /// Retrieve documents with filtering, field selection, and pagination.
    ///
    /// This is the advanced document retrieval method that supports:
    /// - `ids` - Fetch specific documents by their external IDs
    /// - `filter` - Use a filter expression to select matching documents
    /// - `fields` - Select specific fields to return
    /// - `sort` - Sort expressions (applied to filtered/all documents)
    /// - `offset` / `limit` - Pagination
    /// - `retrieve_vectors` - Include vector data (stubbed for now)
    pub fn get_documents_with_options(&self, options: &GetDocumentsOptions) -> Result<DocumentsResult> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        // Determine which fields to display
        let displayed_fields = if let Some(ref field_names) = options.fields {
            let attrs: BTreeSet<String> = field_names.iter().cloned().collect();
            self.get_displayed_fields(&rtxn, &fields_ids_map, Some(&attrs))?
        } else {
            self.get_displayed_fields(&rtxn, &fields_ids_map, None)?
        };

        // If specific IDs are requested, resolve them directly
        if let Some(ref ids) = options.ids {
            let external_ids = self.inner.external_documents_ids();
            let mut internal_ids = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(internal) = external_ids.get(&rtxn, id).map_err(Error::Heed)? {
                    internal_ids.push(internal);
                }
            }

            let total = internal_ids.len() as u64;
            let paginated: Vec<u32> = internal_ids
                .into_iter()
                .skip(options.offset)
                .take(options.limit)
                .collect();

            if paginated.is_empty() {
                return Ok(DocumentsResult {
                    documents: Vec::new(),
                    total,
                    offset: options.offset,
                    limit: options.limit,
                });
            }

            let documents = self.inner.documents(&rtxn, paginated).map_err(Error::Milli)?;
            let mut docs = Vec::with_capacity(documents.len());
            for (_id, obkv) in documents {
                let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                    .map_err(Error::Milli)?;
                docs.push(Value::Object(json));
            }

            return Ok(DocumentsResult {
                documents: docs,
                total,
                offset: options.offset,
                limit: options.limit,
            });
        }

        // Get candidate document IDs (filtered or all)
        let gdwo_filter_owned;
        let candidate_ids = if let Some(filter_val) = &options.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                gdwo_filter_owned = fs;
                if let Some(filter) = Filter::from_str(&gdwo_filter_owned).map_err(Error::Milli)? {
                    filter.evaluate(&rtxn, &self.inner).map_err(Error::Milli)?
                } else {
                    self.inner.documents_ids(&rtxn).map_err(Error::Heed)?
                }
            } else {
                self.inner.documents_ids(&rtxn).map_err(Error::Heed)?
            }
        } else {
            self.inner.documents_ids(&rtxn).map_err(Error::Heed)?
        };

        let total = candidate_ids.len();

        // If sort is specified, fetch all candidate docs, sort, then paginate
        if let Some(ref sort_exprs) = options.sort {
            if !sort_exprs.is_empty() {
                // Use a search with no query to apply sort on the candidate set
                let progress = milli::progress::Progress::default();
                let mut search = self.inner.search(&rtxn, &progress);

                // Apply the filter again to scope the search
                let sort_filter_owned;
                if let Some(filter_val) = &options.filter {
                    if let Some(fs) = parse_filter_to_string(filter_val)? {
                        sort_filter_owned = fs;
                        if let Some(filter) = Filter::from_str(&sort_filter_owned).map_err(Error::Milli)? {
                            search.filter(filter);
                        }
                    }
                }

                // Apply sort criteria
                let mut criteria = Vec::with_capacity(sort_exprs.len());
                for s in sort_exprs {
                    let asc_desc = AscDesc::from_str(s)
                        .map_err(|e| Error::InvalidSort(format!("{e}")))?;
                    criteria.push(asc_desc);
                }
                search.sort_criteria(criteria);
                search.offset(options.offset);
                search.limit(options.limit);

                let result = search.execute().map_err(Error::Milli)?;
                let documents = self.inner.documents(&rtxn, result.documents_ids).map_err(Error::Milli)?;

                let mut docs = Vec::with_capacity(documents.len());
                for (_id, obkv) in documents {
                    let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                        .map_err(Error::Milli)?;
                    docs.push(Value::Object(json));
                }

                return Ok(DocumentsResult {
                    documents: docs,
                    total,
                    offset: options.offset,
                    limit: options.limit,
                });
            }
        }

        let paginated_ids: Vec<u32> = candidate_ids
            .iter()
            .skip(options.offset)
            .take(options.limit)
            .collect();

        if paginated_ids.is_empty() {
            return Ok(DocumentsResult {
                documents: Vec::new(),
                total,
                offset: options.offset,
                limit: options.limit,
            });
        }

        // Fetch the documents
        let documents = self.inner.documents(&rtxn, paginated_ids).map_err(Error::Milli)?;

        let mut docs = Vec::with_capacity(documents.len());
        for (_id, obkv) in documents {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            docs.push(Value::Object(json));
        }

        Ok(DocumentsResult {
            documents: docs,
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

        let doc_count = self.inner.number_of_documents(&wtxn).map_err(Error::Milli)?;
        if doc_count > 0 {
            return Err(Error::PrimaryKeyAlreadyPresent);
        }

        let indexer_config = IndexerConfig::default();
        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.set_primary_key(primary_key.to_string());

        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::default();

        milli_settings
            .execute(&|| false, &progress, &ip_policy, embedder_stats)
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
