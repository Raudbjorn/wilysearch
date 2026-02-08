use milli::documents::{DocumentsBatchBuilder, DocumentsBatchReader, PrimaryKey};
use milli::documents::validate_document_id_value;
use milli::progress::EmbedderStats;
use milli::score_details::{ScoreDetails, ScoringStrategy};
use milli::update::{IndexerConfig, InnerIndexSettings};
use milli::{AscDesc, FieldId, FieldsIdsMap, Filter, FacetDistribution, OrderBy, SearchForFacetValues, Similar, TermsMatchingStrategy};
use milli::tokenizer::{Language, TokenizerBuilder};
use milli::{MatcherBuilder, FormatOptions};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tracing::instrument;

use crate::core::error::{Error, Result};
use crate::core::search::{
    FacetHit, FacetSearchQuery, FacetSearchResult, FacetStats, GetDocumentsOptions, HitsInfo,
    HybridSearchQuery, HybridSearchResult, MatchingStrategy, SearchHit, SearchQuery, SearchResult,
    SimilarQuery, SimilarResult,
};
use crate::core::settings::{
    read_settings_from_index, EmbedderSettings, FacetingSettings, LocalizedAttributeRule,
    PaginationSettings, ProximityPrecision, Settings, SettingsApplier, TypoToleranceSettings,
};
use crate::core::vector::VectorStore;

/// Convert a primary key JSON value to the string form milli uses for external ID mapping.
fn pk_value_to_string(val: &Value) -> String {
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

    /// Parse all vectors from a `_vectors` JSON value.
    ///
    /// Supports three formats per embedder:
    /// - `{"embedder": [f32, ...]}` -- single vector
    /// - `{"embedder": [[f32, ...], ...]}` -- multi-vector
    /// - `{"embedder": {"embeddings": [...], "regenerate": bool}}` -- structured
    ///
    /// Returns all vectors from all embedders flattened into one list.
    fn parse_vectors_value(vectors_val: &Value) -> Result<Vec<Vec<f32>>> {
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
    fn extract_pending_vectors(
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
    fn sync_pending_vectors_post_commit(
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
        let mut wtxn = self.inner.write_txn().map_err(|e| Error::Heed(e))?;

        // If a primary key is provided and the index doesn't have one yet, set it
        if let Some(pk) = primary_key {
            let existing_pk = self.inner.primary_key(&wtxn).map_err(Error::Heed)?;
            if existing_pk.is_none() {
                let indexer_config = IndexerConfig::default();
                let mut settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
                settings.set_primary_key(pk.to_string());
                let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
                let embedder_stats = Arc::new(EmbedderStats::default());
                let progress = milli::progress::Progress::default();
                settings.execute(&|| false, &progress, &ip_policy, embedder_stats)
                    .map_err(Error::Milli)?;
            }
        }

        // Extract pending vectors before the batch loop consumes documents.
        // Use the provided PK or read the one already set on the index.
        let pk_name = match primary_key {
            Some(pk) => Some(pk.to_string()),
            None => self.inner.primary_key(&wtxn).map_err(Error::Heed)?.map(|s| s.to_string()),
        };
        let pending_vectors = Self::extract_pending_vectors(&documents, pk_name.as_deref())?;

        let config = milli::update::IndexDocumentsConfig::default();
        let indexer_config = IndexerConfig::default();

        let embedder_stats = Arc::new(EmbedderStats::default());
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();

        let builder = milli::update::IndexDocuments::new(
            &mut wtxn,
            &self.inner,
            &indexer_config,
            config,
            move |_| (),
            || false,
            &embedder_stats,
            &ip_policy,
        )
        .map_err(Error::Milli)?;

        // Convert JSON documents to DocumentsBatchReader
        let mut batch_builder = DocumentsBatchBuilder::new(Vec::new());
        for doc in documents {
            if let Value::Object(obj) = doc {
                batch_builder
                    .append_json_object(&obj)
                    .map_err(|e| Error::Internal(e.to_string()))?;
            } else {
                return Err(Error::Internal("Document must be a JSON object".to_string()));
            }
        }
        let vector = batch_builder.into_inner().map_err(|e| Error::Internal(e.to_string()))?;
        let reader = DocumentsBatchReader::from_reader(std::io::Cursor::new(vector))
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Add documents
        let (builder, _user_result) = builder.add_documents(reader).map_err(Error::Milli)?;

        // Execute
        builder.execute().map_err(Error::Milli)?;

        wtxn.commit().map_err(|e| Error::Heed(e))?;

        // Sync extracted vectors to the external VectorStore AFTER LMDB commit
        // so that LMDB remains the source of truth. If vector sync fails the
        // documents are still safely persisted and vectors can be re-synced.
        self.sync_pending_vectors_post_commit(pending_vectors)?;

        Ok(())
    }

    /// Update (partially merge) documents into the index.
    ///
    /// Unlike `add_documents`, this uses `IndexDocumentsMethod::UpdateDocuments`
    /// which merges the provided fields into existing documents rather than
    /// replacing them entirely. Only the fields present in the new document
    /// are overwritten; other existing fields are preserved.
    pub fn update_documents(&self, documents: Vec<Value>, primary_key: Option<&str>) -> Result<()> {
        use milli::update::IndexDocumentsMethod;

        let mut wtxn = self.inner.write_txn().map_err(|e| Error::Heed(e))?;

        // If a primary key is provided and the index doesn't have one yet, set it
        if let Some(pk) = primary_key {
            let existing_pk = self.inner.primary_key(&wtxn).map_err(Error::Heed)?;
            if existing_pk.is_none() {
                let indexer_config = IndexerConfig::default();
                let mut settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
                settings.set_primary_key(pk.to_string());
                let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
                let embedder_stats = Arc::new(EmbedderStats::default());
                let progress = milli::progress::Progress::default();
                settings.execute(&|| false, &progress, &ip_policy, embedder_stats)
                    .map_err(Error::Milli)?;
            }
        }

        // Extract pending vectors before the batch loop consumes documents.
        let pk_name = match primary_key {
            Some(pk) => Some(pk.to_string()),
            None => self.inner.primary_key(&wtxn).map_err(Error::Heed)?.map(|s| s.to_string()),
        };
        let pending_vectors = Self::extract_pending_vectors(&documents, pk_name.as_deref())?;

        let config = milli::update::IndexDocumentsConfig {
            update_method: IndexDocumentsMethod::UpdateDocuments,
            ..Default::default()
        };
        let indexer_config = IndexerConfig::default();

        let embedder_stats = Arc::new(EmbedderStats::default());
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();

        let builder = milli::update::IndexDocuments::new(
            &mut wtxn,
            &self.inner,
            &indexer_config,
            config,
            move |_| (),
            || false,
            &embedder_stats,
            &ip_policy,
        )
        .map_err(Error::Milli)?;

        // Convert JSON documents to DocumentsBatchReader
        let mut batch_builder = DocumentsBatchBuilder::new(Vec::new());
        for doc in documents {
            if let Value::Object(obj) = doc {
                batch_builder
                    .append_json_object(&obj)
                    .map_err(|e| Error::Internal(e.to_string()))?;
            } else {
                return Err(Error::Internal("Document must be a JSON object".to_string()));
            }
        }
        let vector = batch_builder.into_inner().map_err(|e| Error::Internal(e.to_string()))?;
        let reader = DocumentsBatchReader::from_reader(std::io::Cursor::new(vector))
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Add documents (using UpdateDocuments method)
        let (builder, _user_result) = builder.add_documents(reader).map_err(Error::Milli)?;

        // Execute
        builder.execute().map_err(Error::Milli)?;

        wtxn.commit().map_err(|e| Error::Heed(e))?;

        // Sync extracted vectors AFTER LMDB commit (see add_documents)
        self.sync_pending_vectors_post_commit(pending_vectors)?;

        Ok(())
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Determine which fields to display based on index settings and query parameters.
    fn get_displayed_fields(
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
    fn parse_locales(&self, locales: Option<&[String]>) -> Result<Option<Vec<Language>>> {
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
    // Search Methods
    // ========================================================================

    /// Perform a search using a SearchQuery and return a SearchResult.
    ///
    /// This is the primary search method that supports all search options including
    /// filtering, pagination, attribute selection, and ranking scores.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::SearchQuery;
    ///
    /// let query = SearchQuery::new("search terms")
    ///     .with_limit(10)
    ///     .with_filter("category = 'books'")
    ///     .with_ranking_score(true);
    ///
    /// let result = index.search(&query)?;
    /// for hit in result.hits {
    ///     println!("Score: {:?}, Doc: {}", hit.ranking_score, hit.document);
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    #[instrument(skip(self, query))]
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResult> {
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::default();

        let mut search = self.inner.search(&rtxn, &progress);

        // Set query string if provided
        if let Some(q) = &query.q {
            search.query(q);
        }

        // ------------------------------------------------------------------
        // Pagination: page-based vs offset/limit
        // ------------------------------------------------------------------
        let use_page_pagination = query.page.is_some() || query.hits_per_page.is_some();
        let (effective_offset, effective_limit, page_val, hpp_val) = if use_page_pagination {
            let page = query.page.unwrap_or(1);
            let hits_per_page = query.hits_per_page.unwrap_or(20);
            let computed_offset = page.saturating_sub(1) * hits_per_page;
            (computed_offset, hits_per_page, page, hits_per_page)
        } else {
            (query.offset, query.limit, 0, 0)
        };

        search.offset(effective_offset);
        search.limit(effective_limit);

        // For page-based pagination we need exhaustive counts
        if use_page_pagination {
            search.exhaustive_number_hits(true);
        }

        // ------------------------------------------------------------------
        // Scoring strategy
        // ------------------------------------------------------------------
        if query.show_ranking_score || query.show_ranking_score_details {
            search.scoring_strategy(ScoringStrategy::Detailed);
        }

        // ------------------------------------------------------------------
        // Terms matching strategy
        // ------------------------------------------------------------------
        let tms = match query.matching_strategy {
            MatchingStrategy::Last => TermsMatchingStrategy::Last,
            MatchingStrategy::All => TermsMatchingStrategy::All,
            MatchingStrategy::Frequency => TermsMatchingStrategy::Frequency,
        };
        search.terms_matching_strategy(tms);

        // ------------------------------------------------------------------
        // Filter
        // ------------------------------------------------------------------
        if let Some(filter_val) = &query.filter {
            if let Some(filter_str) = filter_val.as_str() {
                if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
                    search.filter(filter);
                }
            }
        }

        // ------------------------------------------------------------------
        // Sort criteria
        // ------------------------------------------------------------------
        if let Some(sort_strings) = &query.sort {
            let mut criteria = Vec::with_capacity(sort_strings.len());
            for s in sort_strings {
                let asc_desc = AscDesc::from_str(s)
                    .map_err(|e| Error::InvalidSort(format!("{e}")))?;
                criteria.push(asc_desc);
            }
            search.sort_criteria(criteria);
        }

        // ------------------------------------------------------------------
        // Distinct
        // ------------------------------------------------------------------
        if let Some(ref distinct) = query.distinct {
            search.distinct(distinct.clone());
        }

        // ------------------------------------------------------------------
        // Ranking score threshold
        // ------------------------------------------------------------------
        if let Some(threshold) = query.ranking_score_threshold {
            search.ranking_score_threshold(threshold);
        }

        // ------------------------------------------------------------------
        // Attributes to search on
        // ------------------------------------------------------------------
        let searchable_attrs_owned;
        if let Some(ref attrs) = query.attributes_to_search_on {
            searchable_attrs_owned = attrs.clone();
            search.searchable_attributes(&searchable_attrs_owned);
        }

        // ------------------------------------------------------------------
        // Locales
        // ------------------------------------------------------------------
        let parsed_locales = self.parse_locales(query.locales.as_deref())?;
        if let Some(ref locales) = parsed_locales {
            search.locales(locales.clone());
        }

        // ------------------------------------------------------------------
        // Execute search
        // ------------------------------------------------------------------
        let result = search.execute().map_err(Error::Milli)?;

        // Get the documents -- preserve milli IDs for hybrid search merging
        let returned_doc_ids = result.documents_ids.clone();
        let documents = self.inner.documents(&rtxn, result.documents_ids.clone()).map_err(Error::Milli)?;
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        // Determine which fields to display
        let displayed_fields = self.get_displayed_fields(
            &rtxn,
            &fields_ids_map,
            query.attributes_to_retrieve.as_ref(),
        )?;

        // ------------------------------------------------------------------
        // Highlighting / formatting / matches_position setup
        // ------------------------------------------------------------------
        let needs_formatting = query.attributes_to_highlight.is_some()
            || query.attributes_to_crop.is_some();
        let needs_matches = query.show_matches_position;
        let needs_matcher = needs_formatting || needs_matches;

        // Build MatcherBuilder if needed (consumes matching_words from result)
        let tokenizer;
        let mut matcher_builder_storage;
        let matcher_builder_opt: Option<&MatcherBuilder<'_>> = if needs_matcher {
            tokenizer = TokenizerBuilder::default().into_tokenizer();
            matcher_builder_storage = MatcherBuilder::new(result.matching_words, tokenizer);
            matcher_builder_storage.crop_marker(query.crop_marker.clone());
            matcher_builder_storage.highlight_prefix(query.highlight_pre_tag.clone());
            matcher_builder_storage.highlight_suffix(query.highlight_post_tag.clone());
            Some(&matcher_builder_storage)
        } else {
            None
        };

        // Compute which fields should be highlighted/cropped
        let highlight_fields: BTreeSet<String> = query
            .attributes_to_highlight
            .as_ref()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        let crop_fields: Vec<String> = query
            .attributes_to_crop
            .clone()
            .unwrap_or_default();

        // ------------------------------------------------------------------
        // Build hits
        // ------------------------------------------------------------------
        let mut hits = Vec::with_capacity(documents.len());
        for (idx, (_id, obkv)) in documents.into_iter().enumerate() {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            let doc = Value::Object(json.clone());

            // Ranking score
            let ranking_score = if query.show_ranking_score {
                result
                    .document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::global_score(scores.iter()))
            } else {
                None
            };

            // Ranking score details
            let ranking_score_details = if query.show_ranking_score_details {
                result
                    .document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::to_json_map(scores.iter()))
            } else {
                None
            };

            // Formatting and matches_position
            let mut formatted = None;
            let mut matches_position = None;

            if let Some(mb) = matcher_builder_opt {
                let mut formatted_doc = json.clone();
                let mut all_matches: BTreeMap<String, Vec<crate::core::search::MatchBounds>> =
                    BTreeMap::new();

                for (field_name, value) in &json {
                    if let Value::String(text) = value {
                        let should_highlight = highlight_fields.contains("*")
                            || highlight_fields.contains(field_name);
                        // Cap crop length at 10_000 words to prevent degenerate
                        // allocations from absurd user-supplied values.
                        const MAX_CROP_LENGTH: usize = 10_000;
                        let crop_len = crop_fields.iter().find_map(|c| {
                            let parts: Vec<&str> = c.splitn(2, ':').collect();
                            if parts[0] == "*" || parts[0] == field_name {
                                if parts.len() > 1 {
                                    parts[1].parse::<usize>().ok().map(|v| v.min(MAX_CROP_LENGTH))
                                } else {
                                    Some(query.crop_length.min(MAX_CROP_LENGTH))
                                }
                            } else {
                                None
                            }
                        });

                        let format_options = FormatOptions {
                            highlight: should_highlight,
                            crop: crop_len,
                        };

                        if format_options.should_format() {
                            let mut matcher = mb.build(
                                text,
                                parsed_locales.as_deref(),
                            );
                            let formatted_text = matcher.format(format_options);
                            formatted_doc.insert(
                                field_name.clone(),
                                Value::String(formatted_text.into_owned()),
                            );
                        }

                        if needs_matches {
                            let mut matcher = mb.build(
                                text,
                                parsed_locales.as_deref(),
                            );
                            let bounds = matcher.matches(&[]);
                            if !bounds.is_empty() {
                                let converted: Vec<crate::core::search::MatchBounds> = bounds
                                    .into_iter()
                                    .map(|b| crate::core::search::MatchBounds {
                                        start: b.start,
                                        length: b.length,
                                    })
                                    .collect();
                                all_matches.insert(field_name.clone(), converted);
                            }
                        }
                    }
                }

                if needs_formatting {
                    formatted = Some(Value::Object(formatted_doc));
                }
                if needs_matches && !all_matches.is_empty() {
                    matches_position = Some(all_matches);
                }
            }

            let mut hit = SearchHit::new(doc, ranking_score);
            hit.ranking_score_details = ranking_score_details;
            hit.formatted = formatted;
            hit.matches_position = matches_position;

            hits.push(hit);
        }

        let processing_time_ms = start_time.elapsed().as_millis();
        // RoaringBitmap::len() returns u64; on 64-bit targets this is lossless.
        // On 32-bit targets this would truncate, but wilysearch targets x86_64.
        let total_hits = result.candidates.len() as usize;
        let query_string = query.q.clone().unwrap_or_default();

        // ------------------------------------------------------------------
        // Build result (page-based or offset/limit)
        // ------------------------------------------------------------------
        let mut search_result = if use_page_pagination {
            let mut r = SearchResult::new_paginated(
                hits,
                query_string,
                processing_time_ms,
                total_hits,
                page_val,
                hpp_val,
            );
            r.document_ids = returned_doc_ids;
            r
        } else {
            SearchResult::with_document_ids(
                hits,
                returned_doc_ids,
                query_string,
                processing_time_ms,
                total_hits,
                effective_limit,
                effective_offset,
            )
        };

        // ------------------------------------------------------------------
        // Facet distribution and stats
        // ------------------------------------------------------------------
        if let Some(ref facet_names) = query.facets {
            let mut fd = FacetDistribution::new(&rtxn, &self.inner);
            let facets_with_order: Vec<(String, OrderBy)> = facet_names
                .iter()
                .map(|name| (name.clone(), OrderBy::Lexicographic))
                .collect();
            fd.facets(facets_with_order);
            fd.candidates(result.candidates.clone());

            let distribution = fd.execute().map_err(Error::Milli)?;
            let stats = fd.compute_stats().map_err(Error::Milli)?;

            search_result.facet_distribution = Some(distribution);

            if !stats.is_empty() {
                let facet_stats: BTreeMap<String, FacetStats> = stats
                    .into_iter()
                    .map(|(name, (min, max))| (name, FacetStats { min, max }))
                    .collect();
                search_result.facet_stats = Some(facet_stats);
            }
        }

        Ok(search_result)
    }

    /// Simple search method that returns raw JSON documents.
    ///
    /// This is a convenience method for simple searches. For more control,
    /// use [`search`](Self::search) with a [`SearchQuery`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// let docs = index.search_simple("hello world")?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn search_simple(&self, query: &str) -> Result<Vec<Value>> {
        let search_query = SearchQuery::new(query);
        let result = self.search(&search_query)?;
        Ok(result.hits.into_iter().map(|h| h.document).collect())
    }

    /// Search using a raw embedding vector via the external vector store.
    ///
    /// Returns up to `limit` documents sorted by vector similarity.
    /// If no vector store is configured, returns an empty list.
    pub fn search_vectors(&self, vector: &[f32], limit: usize) -> Result<Vec<Value>> {
        if let Some(store) = &self.vector_store {
            let results =
                store.search(vector, limit, None).map_err(|e| Error::Internal(e.to_string()))?;

            let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
            let ids: Vec<u32> = results.iter().map(|(id, _)| *id).collect();
            let documents = self.inner.documents(&rtxn, ids).map_err(Error::Milli)?;
            let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
            let displayed_fields = self
                .inner
                .displayed_fields_ids(&rtxn)
                .map_err(Error::Milli)?
                .map(|fields| fields.into_iter().collect::<Vec<_>>())
                .unwrap_or_else(|| fields_ids_map.ids().collect());

            let mut docs = Vec::new();
            for (_id, obkv) in documents {
                let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                    .map_err(Error::Milli)?;
                docs.push(Value::Object(json));
            }
            Ok(docs)
        } else {
            Ok(Vec::new())
        }
    }

    /// Perform a hybrid search combining keyword and vector search.
    ///
    /// The `semantic_ratio` controls the balance between keyword and semantic search:
    /// - 0.0 = pure keyword search
    /// - 1.0 = pure semantic/vector search
    /// - 0.5 = balanced hybrid (default)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// # let embedding_vector = vec![0.1_f32; 384];
    /// use wilysearch::core::search::HybridSearchQuery;
    ///
    /// let query = HybridSearchQuery::new("search terms")
    ///     .with_vector(embedding_vector)
    ///     .with_semantic_ratio(0.7)  // Favor semantic search
    ///     .with_limit(10);
    ///
    /// let result = index.hybrid_search(&query)?;
    /// println!("Semantic hits: {:?}", result.semantic_hit_count);
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn hybrid_search(&self, query: &HybridSearchQuery) -> Result<HybridSearchResult> {
        let start_time = Instant::now();

        // If we have a vector store and a vector (or can embed the query), do hybrid search
        if let Some(store) = &self.vector_store {
            // Get the vector for semantic search
            let vector = match &query.vector {
                Some(v) => v.clone(),
                None => {
                    // If no vector provided, we'd need an embedder to generate one
                    // For now, fall back to keyword-only search
                    return self.hybrid_fallback_to_keyword(query, start_time);
                }
            };

            // Perform keyword search
            let keyword_result = self.search(&query.search)?;

            // Perform vector search
            let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;

            // Get filter candidates if filter is specified
            let filter_bitmap = if let Some(filter_val) = &query.search.filter {
                if let Some(filter_str) = filter_val.as_str() {
                    if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
                        Some(filter.evaluate(&rtxn, &self.inner).map_err(Error::Milli)?)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let vector_results = store
                .search(&vector, query.search.limit + query.search.offset, filter_bitmap.as_ref())
                .map_err(|e| Error::Internal(e.to_string()))?;

            // Merge results based on semantic_ratio
            let merged = self.merge_hybrid_results(
                &rtxn,
                keyword_result.clone(),
                vector_results.clone(),
                query.semantic_ratio,
                query.search.limit,
                query.search.offset,
                query.search.show_ranking_score,
                query.search.attributes_to_retrieve.as_ref(),
            )?;

            let processing_time_ms = start_time.elapsed().as_millis();
            let query_string = query.search.q.clone().unwrap_or_default();

            // Count semantic hits
            let semantic_hit_count = merged.1;

            let keyword_estimated = match &keyword_result.hits_info {
                crate::core::search::HitsInfo::OffsetLimit { estimated_total_hits, .. } => *estimated_total_hits,
                crate::core::search::HitsInfo::Pagination { total_hits, .. } => *total_hits,
            };

            Ok(HybridSearchResult::new(
                SearchResult::new(
                    merged.0,
                    query_string,
                    processing_time_ms,
                    keyword_estimated.max(vector_results.len()),
                    query.search.limit,
                    query.search.offset,
                ),
                Some(semantic_hit_count),
            ))
        } else {
            // No vector store, fall back to keyword search
            self.hybrid_fallback_to_keyword(query, start_time)
        }
    }

    /// Fallback to keyword-only search when hybrid search isn't possible.
    fn hybrid_fallback_to_keyword(
        &self,
        query: &HybridSearchQuery,
        start_time: Instant,
    ) -> Result<HybridSearchResult> {
        let keyword_result = self.search(&query.search)?;
        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(HybridSearchResult::new(
            SearchResult {
                processing_time_ms,
                ..keyword_result
            },
            Some(0), // No semantic hits
        ))
    }

    /// Merge keyword and vector search results based on semantic ratio.
    #[allow(clippy::too_many_arguments)]
    fn merge_hybrid_results(
        &self,
        rtxn: &milli::heed::RoTxn<'_>,
        keyword_result: SearchResult,
        vector_results: Vec<(u32, f32)>,
        semantic_ratio: f32,
        limit: usize,
        offset: usize,
        show_ranking_score: bool,
        attributes_to_retrieve: Option<&BTreeSet<String>>,
    ) -> Result<(Vec<SearchHit>, u32)> {
        let fields_ids_map = self.inner.fields_ids_map(rtxn).map_err(Error::Heed)?;
        let displayed_fields = self.get_displayed_fields(rtxn, &fields_ids_map, attributes_to_retrieve)?;

        // Create scored entries for both result sets
        #[derive(Debug)]
        struct ScoredEntry {
            doc_id: u32,
            keyword_score: Option<f64>,
            #[allow(dead_code)]
            vector_score: Option<f32>,
            combined_score: f64,
            source: EntrySource,
        }

        #[derive(Debug, Clone, Copy)]
        enum EntrySource {
            Keyword,
            Vector,
            Both,
        }

        let keyword_weight = 1.0 - semantic_ratio as f64;
        let semantic_weight = semantic_ratio as f64;

        // Build a map of doc_id -> scores
        let mut score_map: std::collections::HashMap<u32, ScoredEntry> =
            std::collections::HashMap::new();

        // Add keyword results using the actual milli document IDs
        for (idx, hit) in keyword_result.hits.iter().enumerate() {
            let doc_id = keyword_result
                .document_ids
                .get(idx)
                .copied()
                .unwrap_or(idx as u32);

            let keyword_score = hit.ranking_score.unwrap_or(1.0);
            let combined = keyword_score * keyword_weight;

            score_map.insert(
                doc_id,
                ScoredEntry {
                    doc_id,
                    keyword_score: Some(keyword_score),
                    vector_score: None,
                    combined_score: combined,
                    source: EntrySource::Keyword,
                },
            );
        }

        // Add vector results
        let mut semantic_hit_count = 0u32;
        for (doc_id, similarity) in &vector_results {
            let vector_score = *similarity as f64;

            if let Some(entry) = score_map.get_mut(doc_id) {
                // Document found in both results
                entry.vector_score = Some(*similarity);
                entry.combined_score =
                    entry.keyword_score.unwrap_or(0.0) * keyword_weight + vector_score * semantic_weight;
                entry.source = EntrySource::Both;
            } else {
                // Document only in vector results
                score_map.insert(
                    *doc_id,
                    ScoredEntry {
                        doc_id: *doc_id,
                        keyword_score: None,
                        vector_score: Some(*similarity),
                        combined_score: vector_score * semantic_weight,
                        source: EntrySource::Vector,
                    },
                );
            }
        }

        // Sort by combined score (descending)
        let mut entries: Vec<_> = score_map.into_values().collect();
        entries.sort_by(|a, b| b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal));

        // Apply offset and limit
        let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

        // Fetch documents and build hits
        let doc_ids: Vec<u32> = entries.iter().map(|e| e.doc_id).collect();
        let documents = self.inner.documents(rtxn, doc_ids).map_err(Error::Milli)?;

        let mut hits = Vec::with_capacity(entries.len());
        let mut doc_map: std::collections::HashMap<u32, _> = documents.into_iter().collect();

        for entry in &entries {
            if let Some(obkv) = doc_map.remove(&entry.doc_id) {
                let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                    .map_err(Error::Milli)?;

                let ranking_score = if show_ranking_score {
                    Some(entry.combined_score)
                } else {
                    None
                };

                hits.push(SearchHit::new(Value::Object(json), ranking_score));

                // Count semantic hits (documents that came from vector search)
                if matches!(entry.source, EntrySource::Vector | EntrySource::Both) {
                    semantic_hit_count += 1;
                }
            }
        }

        Ok((hits, semantic_hit_count))
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
        use milli::update::new::indexer;
        use milli::update::InnerIndexSettings;

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
                if let Some(internal_id) = external_ids.get(&rtxn, id).ok().flatten() {
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

            let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
            let embedders = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
                .map_err(Error::Milli)?
                .runtime_embedders;

            let mut indexer_ops = indexer::IndexOperations::new();
            let id_refs: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
            indexer_ops.delete_documents(&id_refs);

            let indexer_alloc = Bump::new();
            let (document_changes, operation_stats, primary_key) = indexer_ops
                .into_changes(
                    &indexer_alloc,
                    &self.inner,
                    &rtxn,
                    None,
                    &mut new_fields_ids_map,
                    &|| false,
                    milli::progress::Progress::default(),
                    None,
                )
                .map_err(Error::Milli)?;

            if let Some(error) = operation_stats.into_iter().find_map(|stat| stat.error) {
                return Err(Error::Milli(error.into()));
            }

            let grenad_params = indexer_config.grenad_parameters();
            let indexing_pool = milli::ThreadPoolNoAbortBuilder::new()
                .build()
                .map_err(|e| Error::Internal(format!("failed to build indexing thread pool: {e}")))?;

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
                    &|| false,
                    &milli::progress::Progress::default(),
                    &ip_policy,
                    &Default::default(),
                )
            })
            .map_err(|e| Error::Internal(e.to_string()))?
            .map_err(Error::Milli)?;
        } // rtxn + document_changes dropped here, before commit

        // Remove vectors from the external VectorStore
        if let Some(store) = &self.vector_store {
            if !vector_ids_to_remove.is_empty() {
                store
                    .remove_documents(&vector_ids_to_remove)
                    .map_err(|e| Error::VectorStore(e.to_string()))?;
            }
        }

        wtxn.commit().map_err(Error::Heed)?;

        Ok(existing_count)
    }

    /// Delete documents matching a filter expression.
    ///
    /// The filter uses the same syntax as search filters (e.g., "genre = 'horror'" or "year > 2000").
    /// Returns the number of documents deleted.
    ///
    /// Note: The field used in the filter must be configured as filterable in the index settings.
    pub fn delete_by_filter(&self, filter: &str) -> Result<u64> {
        use bumpalo::Bump;
        use milli::update::new::indexer::{self, DocumentDeletion};
        use milli::update::InnerIndexSettings;

        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;

        // Pre-deletion reads use a scoped rtxn. We extract owned data
        // (candidates, count, vector_ids) so the rtxn can be dropped immediately.
        let (candidates, count, vector_ids_to_remove) = {
            let rtxn = self.inner.read_txn().map_err(Error::Heed)?;

            let filter = Filter::from_str(filter)
                .map_err(|e| Error::Internal(format!("Invalid filter: {e}")))?
                .ok_or_else(|| Error::Internal("Empty filter expression".to_string()))?;

            let candidates = filter.evaluate(&rtxn, &self.inner).map_err(Error::Milli)?;
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

            let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
            let embedders =
                InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
                    .map_err(Error::Milli)?
                    .runtime_embedders;

            let primary_key_str = self
                .inner
                .primary_key(&rtxn)
                .map_err(Error::Heed)?
                .ok_or_else(|| {
                    Error::Internal("Index has no primary key configured".to_string())
                })?
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
                    &|| false,
                    &milli::progress::Progress::default(),
                    &ip_policy,
                    &Default::default(),
                )
            })
            .map_err(|e| Error::Internal(e.to_string()))?
            .map_err(Error::Milli)?;
        } // rtxn + embedders + document_changes dropped here, before commit

        // Remove vectors from the external VectorStore
        if let Some(store) = &self.vector_store {
            if !vector_ids_to_remove.is_empty() {
                store
                    .remove_documents(&vector_ids_to_remove)
                    .map_err(|e| Error::VectorStore(e.to_string()))?;
            }
        }

        wtxn.commit().map_err(Error::Heed)?;

        Ok(count)
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
        let candidate_ids = if let Some(filter_val) = &options.filter {
            if let Some(filter_str) = filter_val.as_str() {
                if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
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
                if let Some(filter_val) = &options.filter {
                    if let Some(filter_str) = filter_val.as_str() {
                        if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
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
        // Check if the index already has documents
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        let doc_count = self.inner.number_of_documents(&rtxn).map_err(Error::Milli)?;
        if doc_count > 0 {
            return Err(Error::PrimaryKeyAlreadyPresent);
        }
        drop(rtxn);

        // Use the milli Settings builder to set the primary key
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
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

        // Apply filter
        if let Some(ref filter_val) = query.filter {
            if let Some(filter_str) = filter_val.as_str() {
                if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
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
        let id_string = validate_document_id_value(query.id.clone())
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

        // Apply optional filter
        if let Some(ref filter_val) = query.filter {
            if let Some(filter_str) = filter_val.as_str() {
                if let Some(filter) = Filter::from_str(filter_str).map_err(Error::Milli)? {
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_displayed_fields();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_searchable_fields();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_filterable_fields();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_sortable_fields();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_criteria();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_stop_words();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_non_separator_tokens();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_separator_tokens();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_dictionary();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_synonyms();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_distinct_field();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_proximity_precision();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_authorize_typos();
        milli_settings.reset_min_word_len_one_typo();
        milli_settings.reset_min_word_len_two_typos();
        milli_settings.reset_exact_words();
        milli_settings.reset_exact_attributes();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_max_values_per_facet();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_pagination_max_total_hits();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_embedder_settings();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_search_cutoff();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_localized_attributes_rules();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_facet_search();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings = milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        milli_settings.reset_prefix_search();
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings.execute(&|| false, &milli::progress::Progress::default(), &ip_policy, embedder_stats).map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
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
