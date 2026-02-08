mod hybrid;
mod search;
mod updates;
mod vectors;

use milli::progress::EmbedderStats;
use milli::score_details::ScoreDetails;
use milli::update::{IndexerConfig, InnerIndexSettings};
use milli::{AscDesc, FieldId, FieldsIdsMap, Filter, SearchForFacetValues, Similar, TermsMatchingStrategy};
use milli::tokenizer::Language;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use crate::core::error::{Error, Result};
use crate::core::search::{
    FacetHit, FacetSearchQuery, FacetSearchResult, GetDocumentsOptions, HitsInfo,
    MatchingStrategy, SearchHit,
    SimilarQuery, SimilarResult,
};
use crate::core::settings::{
    read_settings_from_index, EmbedderSettings, FacetingSettings, LocalizedAttributeRule,
    PaginationSettings, ProximityPrecision, Settings, SettingsApplier, TypoToleranceSettings,
};
use crate::core::vector::VectorStore;

/// Parse a filter `Value` into a filter expression string.
///
/// Returns `Ok(None)` for empty/null values.
/// Returns `Err(InvalidFilter)` for unsupported shapes.
pub(crate) fn parse_filter_to_string(filter_val: &Value) -> Result<Option<String>> {
    match filter_val {
        Value::String(s) if s.is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        Value::Array(arr) if arr.is_empty() => Ok(None),
        Value::Array(arr) => {
            let mut and_clauses = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::String(s) => and_clauses.push(s.clone()),
                    Value::Array(inner) => {
                        let or_parts: Vec<&str> = inner
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect();
                        if or_parts.is_empty() {
                            continue;
                        }
                        if or_parts.len() == 1 {
                            and_clauses.push(or_parts[0].to_string());
                        } else {
                            and_clauses.push(format!("({})", or_parts.join(" OR ")));
                        }
                    }
                    _ => {
                        return Err(Error::InvalidFilter(
                            "filter array elements must be strings or arrays of strings".to_string(),
                        ));
                    }
                }
            }
            if and_clauses.is_empty() {
                Ok(None)
            } else {
                Ok(Some(and_clauses.join(" AND ")))
            }
        }
        Value::Null => Ok(None),
        _ => Err(Error::InvalidFilter(
            "filter must be a string or array".to_string(),
        )),
    }
}

/// Convert a primary key JSON value to the string form milli uses for external ID mapping.
pub(crate) fn pk_value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

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

/// A single Meilisearch index backed by milli's LMDB storage.
///
/// Provides methods for adding, updating, deleting, and searching documents,
/// as well as reading and writing index settings.
///
/// Obtain an `Index` via [`Meilisearch::create_index`](crate::Meilisearch::create_index)
/// or [`Meilisearch::get_index`](crate::Meilisearch::get_index).
pub struct Index {
    pub(crate) inner: milli::Index,
    pub(crate) vector_store: Option<Arc<dyn VectorStore>>,
}

impl Index {
    /// Wrap a raw milli index with an optional external vector store.
    pub fn new(inner: milli::Index, vector_store: Option<Arc<dyn VectorStore>>) -> Self {
        Self { inner, vector_store }
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Determine which fields to display based on index settings and query parameters.
    pub(crate) fn get_displayed_fields(
        &self,
        rtxn: &milli::heed::RoTxn<'_>,
        fields_ids_map: &FieldsIdsMap,
        attributes_to_retrieve: Option<&BTreeSet<String>>,
    ) -> Result<Vec<FieldId>> {
        // If specific attributes are requested, use those
        if let Some(attrs) = attributes_to_retrieve {
            let field_ids: Vec<FieldId> = attrs
                .iter()
                .filter_map(|name| fields_ids_map.id(name))
                .collect();
            return Ok(field_ids);
        }

        // Otherwise, use the displayed fields from index settings
        let displayed_fields = self
            .inner
            .displayed_fields_ids(rtxn)
            .map_err(Error::Milli)?
            .map(|fields| fields.into_iter().collect::<Vec<_>>())
            .unwrap_or_else(|| fields_ids_map.ids().collect());

        Ok(displayed_fields)
    }

    /// Parse locale strings into milli Language values.
    pub(crate) fn parse_locales(&self, locales: Option<&[String]>) -> Result<Option<Vec<Language>>> {
        match locales {
            None => Ok(None),
            Some(locale_strs) => {
                let mut langs = Vec::with_capacity(locale_strs.len());
                for s in locale_strs {
                    let locale: meilisearch_types::locales::Locale = s
                        .parse()
                        .map_err(|_| Error::Internal(format!("Unknown locale: {s}")))?;
                    langs.push(Language::from(locale));
                }
                Ok(Some(langs))
            }
        }
    }

    // ========================================================================
    // Document Operations
    // ========================================================================

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

    /// Search within facet values.
    ///
    /// Given a facet name (and optionally a facet query and a search query),
    /// returns matching facet values with their document counts.
    pub fn facet_search(&self, query: &FacetSearchQuery) -> Result<FacetSearchResult> {
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::default();

        // Build the inner keyword search to scope facet results
        let mut inner_search = self.inner.search(&rtxn, &progress);

        if let Some(ref q) = query.q {
            inner_search.query(q);
        }

        // Apply matching strategy
        let tms = match query.matching_strategy {
            MatchingStrategy::Last => TermsMatchingStrategy::Last,
            MatchingStrategy::All => TermsMatchingStrategy::All,
            MatchingStrategy::Frequency => TermsMatchingStrategy::Frequency,
        };
        inner_search.terms_matching_strategy(tms);

        // Apply filter (supports string, array-of-strings, and array-of-arrays)
        let facet_filter_owned;
        if let Some(ref filter_val) = query.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                facet_filter_owned = fs;
                if let Some(filter) = Filter::from_str(&facet_filter_owned).map_err(Error::Milli)? {
                    inner_search.filter(filter);
                }
            }
        }

        // Apply ranking score threshold
        if let Some(threshold) = query.ranking_score_threshold {
            inner_search.ranking_score_threshold(threshold);
        }

        // Apply attributes to search on
        let searchable_attrs_owned;
        if let Some(ref attrs) = query.attributes_to_search_on {
            searchable_attrs_owned = attrs.clone();
            inner_search.searchable_attributes(&searchable_attrs_owned);
        }

        // Build facet search
        let mut facet_search =
            SearchForFacetValues::new(query.facet_name.clone(), inner_search, false);

        if let Some(ref fq) = query.facet_query {
            facet_search.query(fq);
        }

        // Apply locales
        let parsed_locales = self.parse_locales(query.locales.as_deref())?;
        if let Some(locales) = parsed_locales {
            facet_search.locales(locales);
        }

        let facet_hits = facet_search.execute().map_err(Error::Milli)?;

        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(FacetSearchResult {
            facet_hits: facet_hits
                .into_iter()
                .map(|fv| FacetHit {
                    value: fv.value,
                    count: fv.count,
                })
                .collect(),
            facet_query: query.facet_query.clone(),
            processing_time_ms,
        })
    }

    /// Find documents similar to a given document using vector embeddings.
    ///
    /// This method uses the configured embedder to look up the source document's
    /// vector and find nearby documents in embedding space. The source document
    /// is automatically excluded from the results.
    ///
    /// # Arguments
    ///
    /// * `query` - A [`SimilarQuery`] specifying the source document ID, embedder
    ///   name, pagination, optional filter, and score display options.
    ///
    /// # Errors
    ///
    /// * [`Error::EmbedderNotFound`] if the embedder named in the query is not
    ///   configured on this index.
    /// * [`Error::DocumentNotFound`] if the source document ID does not exist.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::search::SimilarQuery;
    /// use serde_json::json;
    ///
    /// let query = SimilarQuery {
    ///     id: json!("doc-42"),
    ///     embedder: "default".to_string(),
    ///     offset: 0,
    ///     limit: 10,
    ///     filter: None,
    ///     attributes_to_retrieve: None,
    ///     retrieve_vectors: false,
    ///     show_ranking_score: true,
    ///     show_ranking_score_details: false,
    ///     ranking_score_threshold: None,
    /// };
    ///
    /// let result = index.get_similar_documents(&query)?;
    /// for hit in &result.hits {
    ///     println!("{}", hit.document);
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn get_similar_documents(&self, query: &SimilarQuery) -> Result<SimilarResult> {
        let start_time = Instant::now();

        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;

        // Resolve the embedder from index settings
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let inner_settings = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
            .map_err(Error::Milli)?;

        let runtime_embedder = inner_settings
            .runtime_embedders
            .get(&query.embedder)
            .ok_or_else(|| Error::EmbedderNotFound(query.embedder.clone()))?;

        let embedder = runtime_embedder.embedder.clone();
        let quantized = runtime_embedder.is_quantized;

        // Convert the JSON document ID to a string, then resolve to an internal ID
        let id_string = milli::documents::validate_document_id_value(query.id.clone())
            .map_err(|e| Error::Internal(format!("Invalid document id: {e}")))?;

        let external_ids = self.inner.external_documents_ids();
        let internal_id = external_ids
            .get(&rtxn, &id_string)
            .map_err(Error::Heed)?
            .ok_or_else(|| Error::DocumentNotFound(id_string.clone()))?;

        // Build the Similar query
        let progress = milli::progress::Progress::default();

        let mut similar = Similar::new(
            internal_id,
            query.offset,
            query.limit,
            &self.inner,
            &rtxn,
            query.embedder.clone(),
            embedder,
            quantized,
            &progress,
        );

        // Apply optional filter (supports string, array-of-strings, and array-of-arrays)
        let similar_filter_owned;
        if let Some(ref filter_val) = query.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                similar_filter_owned = fs;
                if let Some(filter) = Filter::from_str(&similar_filter_owned).map_err(Error::Milli)? {
                    similar.filter(filter);
                }
            }
        }

        // Apply ranking score threshold
        if let Some(threshold) = query.ranking_score_threshold {
            similar.ranking_score_threshold(threshold);
        }

        // Execute the similar search
        let milli::SearchResult {
            documents_ids,
            candidates,
            document_scores,
            ..
        } = similar.execute().map_err(Error::Milli)?;

        // Fetch documents and build hits
        let documents = self
            .inner
            .documents(&rtxn, documents_ids.clone())
            .map_err(Error::Milli)?;
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        let displayed_fields = self.get_displayed_fields(
            &rtxn,
            &fields_ids_map,
            query.attributes_to_retrieve.as_ref(),
        )?;

        let mut hits = Vec::with_capacity(documents.len());
        for (idx, (_doc_id, obkv)) in documents.into_iter().enumerate() {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            let doc = Value::Object(json);

            let ranking_score = if query.show_ranking_score {
                document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::global_score(scores.iter()))
            } else {
                None
            };

            let ranking_score_details = if query.show_ranking_score_details {
                document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::to_json_map(scores.iter()))
            } else {
                None
            };

            let mut hit = SearchHit::new(doc, ranking_score);
            hit.ranking_score_details = ranking_score_details;
            hits.push(hit);
        }

        let processing_time_ms = start_time.elapsed().as_millis();
        let total_hits = candidates.len() as usize;

        Ok(SimilarResult {
            hits,
            id: id_string,
            processing_time_ms,
            hits_info: HitsInfo::OffsetLimit {
                limit: query.limit,
                offset: query.offset,
                estimated_total_hits: total_hits,
            },
        })
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

    // ========================================================================
    // Settings Operations
    // ========================================================================

    /// Get the current settings of the index.
    ///
    /// Returns a [`Settings`] struct containing all configured index settings
    /// including searchable/filterable/sortable attributes, ranking rules,
    /// embedders, and more.
    pub fn get_settings(&self) -> Result<Settings> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        read_settings_from_index(&rtxn, &self.inner)
    }

    /// Update the index settings.
    ///
    /// Only the fields that are set (not `None`) in the provided [`Settings`]
    /// will be updated. Fields set to `None` are left unchanged.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::{Settings, EmbedderSettings};
    ///
    /// let settings = Settings::new()
    ///     .with_searchable_attributes(vec!["title".to_string(), "content".to_string()])
    ///     .with_filterable_attributes(vec!["category".to_string(), "price".to_string()])
    ///     .with_embedder("default", EmbedderSettings::openai("your-api-key"));
    ///
    /// index.update_settings(&settings)?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn update_settings(&self, settings: &Settings) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();

        let milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);

        let applier = SettingsApplier { builder: milli_settings };
        let milli_settings = applier.apply(settings)?;

        // Execute the settings update
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::default();

        milli_settings
            .execute(&|| false, &progress, &ip_policy, embedder_stats)
            .map_err(Error::Milli)?;

        wtxn.commit().map_err(Error::Heed)?;

        Ok(())
    }

    /// Reset all settings to their default values.
    ///
    /// This resets:
    /// - Searchable attributes (all fields become searchable)
    /// - Displayed attributes (all fields become displayed)
    /// - Filterable attributes (cleared)
    /// - Sortable attributes (cleared)
    /// - Ranking rules (reset to default)
    /// - Stop words (cleared)
    /// - Synonyms (cleared)
    /// - Embedders (cleared)
    /// - Distinct attribute (cleared)
    /// - Typo tolerance (reset to default)
    ///
    /// Note: This does NOT delete documents.
    pub fn reset_settings(&self) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();

        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);

        // Reset all settings to their defaults
        milli_settings.reset_searchable_fields();
        milli_settings.reset_displayed_fields();
        milli_settings.reset_filterable_fields();
        milli_settings.reset_sortable_fields();
        milli_settings.reset_criteria();
        milli_settings.reset_stop_words();
        milli_settings.reset_non_separator_tokens();
        milli_settings.reset_separator_tokens();
        milli_settings.reset_dictionary();
        milli_settings.reset_synonyms();
        milli_settings.reset_embedder_settings();
        milli_settings.reset_distinct_field();
        milli_settings.reset_proximity_precision();
        milli_settings.reset_authorize_typos();
        milli_settings.reset_min_word_len_one_typo();
        milli_settings.reset_min_word_len_two_typos();
        milli_settings.reset_exact_words();
        milli_settings.reset_exact_attributes();
        milli_settings.reset_disable_on_numbers();
        milli_settings.reset_max_values_per_facet();
        milli_settings.reset_pagination_max_total_hits();
        milli_settings.reset_search_cutoff();
        milli_settings.reset_localized_attributes_rules();
        milli_settings.reset_facet_search();
        milli_settings.reset_prefix_search();

        // Execute the settings update
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::default();

        milli_settings
            .execute(&|| false, &progress, &ip_policy, embedder_stats)
            .map_err(Error::Milli)?;

        wtxn.commit().map_err(Error::Heed)?;

        Ok(())
    }

    /// Helper: open a write transaction, create a milli settings builder,
    /// apply a single reset closure, execute, and commit.
    fn execute_settings_reset(
        &self,
        apply: impl FnOnce(&mut milli::update::Settings<'_, '_, '_>),
    ) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        apply(&mut milli_settings);
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings
            .execute(
                &|| false,
                &milli::progress::Progress::default(),
                &ip_policy,
                embedder_stats,
            )
            .map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
    }

    // ========================================================================
    // Individual Settings Accessors
    // ========================================================================

    // --- displayed_attributes ---

    /// Get the displayed attributes setting.
    pub fn get_displayed_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.displayed_attributes)
    }

    /// Update the displayed attributes setting.
    pub fn update_displayed_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_displayed_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the displayed attributes to their default value.
    pub fn reset_displayed_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_displayed_fields())
    }

    // --- searchable_attributes ---

    /// Get the searchable attributes setting.
    pub fn get_searchable_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.searchable_attributes)
    }

    /// Update the searchable attributes setting.
    pub fn update_searchable_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_searchable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the searchable attributes to their default value.
    pub fn reset_searchable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_searchable_fields())
    }

    // --- filterable_attributes ---

    /// Get the filterable attributes setting.
    pub fn get_filterable_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.filterable_attributes)
    }

    /// Update the filterable attributes setting.
    pub fn update_filterable_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_filterable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the filterable attributes to their default value.
    pub fn reset_filterable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_filterable_fields())
    }

    // --- sortable_attributes ---

    /// Get the sortable attributes setting.
    pub fn get_sortable_attributes(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.sortable_attributes)
    }

    /// Update the sortable attributes setting.
    pub fn update_sortable_attributes(&self, attrs: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_sortable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the sortable attributes to their default value.
    pub fn reset_sortable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_sortable_fields())
    }

    // --- ranking_rules ---

    /// Get the ranking rules setting.
    pub fn get_ranking_rules(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.ranking_rules)
    }

    /// Update the ranking rules setting.
    pub fn update_ranking_rules(&self, rules: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_ranking_rules(rules);
        self.update_settings(&settings)
    }

    /// Reset the ranking rules to their default value.
    pub fn reset_ranking_rules(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_criteria())
    }

    // --- stop_words ---

    /// Get the stop words setting.
    pub fn get_stop_words(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.stop_words)
    }

    /// Update the stop words setting.
    pub fn update_stop_words(&self, words: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_stop_words(words);
        self.update_settings(&settings)
    }

    /// Reset the stop words to their default value.
    pub fn reset_stop_words(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_stop_words())
    }

    // --- non_separator_tokens ---

    /// Get the non-separator tokens setting.
    pub fn get_non_separator_tokens(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.non_separator_tokens)
    }

    /// Update the non-separator tokens setting.
    pub fn update_non_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_non_separator_tokens(tokens);
        self.update_settings(&settings)
    }

    /// Reset the non-separator tokens to their default value.
    pub fn reset_non_separator_tokens(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_non_separator_tokens())
    }

    // --- separator_tokens ---

    /// Get the separator tokens setting.
    pub fn get_separator_tokens(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.separator_tokens)
    }

    /// Update the separator tokens setting.
    pub fn update_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_separator_tokens(tokens);
        self.update_settings(&settings)
    }

    /// Reset the separator tokens to their default value.
    pub fn reset_separator_tokens(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_separator_tokens())
    }

    // --- dictionary ---

    /// Get the dictionary setting.
    pub fn get_dictionary(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.dictionary)
    }

    /// Update the dictionary setting.
    pub fn update_dictionary(&self, words: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_dictionary(words);
        self.update_settings(&settings)
    }

    /// Reset the dictionary to its default value.
    pub fn reset_dictionary(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_dictionary())
    }

    // --- synonyms ---

    /// Get the synonyms setting.
    pub fn get_synonyms(&self) -> Result<Option<BTreeMap<String, Vec<String>>>> {
        let settings = self.get_settings()?;
        Ok(settings.synonyms)
    }

    /// Update the synonyms setting.
    pub fn update_synonyms(&self, synonyms: BTreeMap<String, Vec<String>>) -> Result<()> {
        let settings = Settings::new().with_synonyms(synonyms);
        self.update_settings(&settings)
    }

    /// Reset the synonyms to their default value.
    pub fn reset_synonyms(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_synonyms())
    }

    // --- distinct_attribute ---

    /// Get the distinct attribute setting.
    pub fn get_distinct_attribute(&self) -> Result<Option<String>> {
        let settings = self.get_settings()?;
        Ok(settings.distinct_attribute)
    }

    /// Update the distinct attribute setting.
    pub fn update_distinct_attribute(&self, attr: String) -> Result<()> {
        let settings = Settings::new().with_distinct_attribute(attr);
        self.update_settings(&settings)
    }

    /// Reset the distinct attribute to its default value.
    pub fn reset_distinct_attribute(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_distinct_field())
    }

    // --- proximity_precision ---

    /// Get the proximity precision setting.
    pub fn get_proximity_precision(&self) -> Result<Option<ProximityPrecision>> {
        let settings = self.get_settings()?;
        Ok(settings.proximity_precision)
    }

    /// Update the proximity precision setting.
    pub fn update_proximity_precision(&self, precision: ProximityPrecision) -> Result<()> {
        let settings = Settings::new().with_proximity_precision(precision);
        self.update_settings(&settings)
    }

    /// Reset the proximity precision to its default value.
    pub fn reset_proximity_precision(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_proximity_precision())
    }

    // --- typo_tolerance ---

    /// Get the typo tolerance setting.
    pub fn get_typo_tolerance(&self) -> Result<Option<TypoToleranceSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.typo_tolerance)
    }

    /// Update the typo tolerance setting.
    pub fn update_typo_tolerance(&self, typo_tolerance: TypoToleranceSettings) -> Result<()> {
        let settings = Settings::new().with_typo_tolerance(typo_tolerance);
        self.update_settings(&settings)
    }

    /// Reset the typo tolerance to its default value.
    pub fn reset_typo_tolerance(&self) -> Result<()> {
        self.execute_settings_reset(|s| {
            s.reset_authorize_typos();
            s.reset_min_word_len_one_typo();
            s.reset_min_word_len_two_typos();
            s.reset_exact_words();
            s.reset_exact_attributes();
        })
    }

    // --- faceting ---

    /// Get the faceting setting.
    pub fn get_faceting(&self) -> Result<Option<FacetingSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.faceting)
    }

    /// Update the faceting setting.
    pub fn update_faceting(&self, faceting: FacetingSettings) -> Result<()> {
        let settings = Settings::new().with_faceting(faceting);
        self.update_settings(&settings)
    }

    /// Reset the faceting to its default value.
    pub fn reset_faceting(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_max_values_per_facet())
    }

    // --- pagination ---

    /// Get the pagination setting.
    pub fn get_pagination(&self) -> Result<Option<PaginationSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.pagination)
    }

    /// Update the pagination setting.
    pub fn update_pagination(&self, pagination: PaginationSettings) -> Result<()> {
        let settings = Settings::new().with_pagination(pagination);
        self.update_settings(&settings)
    }

    /// Reset the pagination to its default value.
    pub fn reset_pagination(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_pagination_max_total_hits())
    }

    // --- embedders ---

    /// Get the embedders setting.
    pub fn get_embedders(&self) -> Result<Option<HashMap<String, EmbedderSettings>>> {
        let settings = self.get_settings()?;
        Ok(settings.embedders)
    }

    /// Update the embedders setting.
    pub fn update_embedders(&self, embedders: HashMap<String, EmbedderSettings>) -> Result<()> {
        let settings = Settings::new().with_embedders(embedders);
        self.update_settings(&settings)
    }

    /// Reset the embedders to their default value.
    pub fn reset_embedders(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_embedder_settings())
    }

    // --- search_cutoff_ms ---

    /// Get the search cutoff in milliseconds setting.
    pub fn get_search_cutoff_ms(&self) -> Result<Option<u64>> {
        let settings = self.get_settings()?;
        Ok(settings.search_cutoff_ms)
    }

    /// Update the search cutoff in milliseconds setting.
    pub fn update_search_cutoff_ms(&self, ms: u64) -> Result<()> {
        let settings = Settings::new().with_search_cutoff_ms(ms);
        self.update_settings(&settings)
    }

    /// Reset the search cutoff to its default value.
    pub fn reset_search_cutoff_ms(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_search_cutoff())
    }

    // --- localized_attributes ---

    /// Get the localized attributes setting.
    pub fn get_localized_attributes(&self) -> Result<Option<Vec<LocalizedAttributeRule>>> {
        let settings = self.get_settings()?;
        Ok(settings.localized_attributes)
    }

    /// Update the localized attributes setting.
    pub fn update_localized_attributes(&self, rules: Vec<LocalizedAttributeRule>) -> Result<()> {
        let settings = Settings::new().with_localized_attributes(rules);
        self.update_settings(&settings)
    }

    /// Reset the localized attributes to their default value.
    pub fn reset_localized_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_localized_attributes_rules())
    }

    // --- facet_search ---

    /// Get the facet search setting.
    pub fn get_facet_search(&self) -> Result<Option<bool>> {
        let settings = self.get_settings()?;
        Ok(settings.facet_search)
    }

    /// Update the facet search setting.
    pub fn update_facet_search(&self, enabled: bool) -> Result<()> {
        let settings = Settings::new().with_facet_search(enabled);
        self.update_settings(&settings)
    }

    /// Reset the facet search to its default value.
    pub fn reset_facet_search(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_facet_search())
    }

    // --- prefix_search ---

    /// Get the prefix search setting.
    pub fn get_prefix_search(&self) -> Result<Option<String>> {
        let settings = self.get_settings()?;
        Ok(settings.prefix_search)
    }

    /// Update the prefix search setting.
    pub fn update_prefix_search(&self, mode: String) -> Result<()> {
        let settings = Settings::new().with_prefix_search(mode);
        self.update_settings(&settings)
    }

    /// Reset the prefix search to its default value.
    pub fn reset_prefix_search(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_prefix_search())
    }

    /// Get the primary key field name for this index.
    ///
    /// Returns `None` if no primary key has been set yet (the index has no documents).
    pub fn primary_key(&self) -> Result<Option<String>> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        let pk = self.inner.primary_key(&rtxn).map_err(Error::Heed)?;
        Ok(pk.map(|s| s.to_string()))
    }
}
