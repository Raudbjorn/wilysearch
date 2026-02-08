//! Synchronous trait definitions for the Meilisearch API surface.
//!
//! # Design Decision: Synchronous API
//!
//! All trait methods are synchronous by design. Although the underlying LMDB
//! operations involve disk I/O and the optional SurrealDB backend involves
//! network I/O, a sync API was chosen because:
//!
//! 1. **Simplicity** -- Embedded use-cases don't benefit from async overhead.
//!    LMDB operations are memory-mapped and effectively in-process.
//! 2. **Compatibility** -- Sync traits compose with any executor (or none).
//!    Callers can wrap in `spawn_blocking()` if needed.
//! 3. **No task queue** -- Unlike server Meilisearch, there's no HTTP layer
//!    or background task system. Operations complete inline.
//!
//! # Error Types
//!
//! [`Result<T>`] uses [`Error`] -- a typed enum with variants for every error
//! category (LMDB, I/O, serialization, index-not-found, etc.). Error details
//! are preserved across the trait boundary; there is no type erasure.
//!
//! # Thread-Blocking Note
//!
//! The optional SurrealDB vector store backend bridges async calls via
//! `tokio::runtime::Runtime::block_on()`. When using `Engine` from within an
//! existing multi-thread tokio runtime, the store uses `block_in_place()` to
//! avoid nesting runtimes. This blocks the current thread. For `current_thread`
//! runtimes, the store panics with a clear error -- use a multi-thread runtime
//! or wrap calls in `spawn_blocking()` instead.

use serde_json::Value;

use crate::types::*;
use crate::error::Error;

pub type Result<T> = std::result::Result<T, Error>;

// ─── Documents ───────────────────────────────────────────────────────────────

pub trait Documents {
    /// GET /indexes/{indexUid}/documents/{documentId}
    fn get_document(
        &self,
        index_uid: &str,
        document_id: &str,
        query: &DocumentQuery,
    ) -> Result<Value>;

    /// GET /indexes/{indexUid}/documents
    fn get_documents(
        &self,
        index_uid: &str,
        query: &DocumentsQuery,
    ) -> Result<DocumentsResponse>;

    /// POST /indexes/{indexUid}/documents/fetch
    fn fetch_documents(
        &self,
        index_uid: &str,
        request: &FetchDocumentsRequest,
    ) -> Result<DocumentsResponse>;

    /// POST /indexes/{indexUid}/documents
    fn add_or_replace_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo>;

    /// PUT /indexes/{indexUid}/documents
    fn add_or_update_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/documents/{documentId}
    fn delete_document(&self, index_uid: &str, document_id: &str) -> Result<TaskInfo>;

    /// POST /indexes/{indexUid}/documents/delete
    fn delete_documents_by_filter(
        &self,
        index_uid: &str,
        request: &DeleteDocumentsByFilterRequest,
    ) -> Result<TaskInfo>;

    /// POST /indexes/{indexUid}/documents/delete-batch
    fn delete_documents_by_batch(
        &self,
        index_uid: &str,
        document_ids: &[Value],
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/documents
    fn delete_all_documents(&self, index_uid: &str) -> Result<TaskInfo>;
}

// ─── Search ──────────────────────────────────────────────────────────────────

pub trait Search {
    /// POST /indexes/{indexUid}/search
    fn search(
        &self,
        index_uid: &str,
        request: &SearchRequest,
    ) -> Result<SearchResponse>;

    /// POST /indexes/{indexUid}/similar
    fn similar(
        &self,
        index_uid: &str,
        request: &SimilarRequest,
    ) -> Result<SimilarResponse>;

    /// POST /multi-search
    fn multi_search(&self, request: &MultiSearchRequest) -> Result<MultiSearchResult>;

    /// POST /indexes/{indexUid}/facet-search
    fn facet_search(
        &self,
        index_uid: &str,
        request: &FacetSearchRequest,
    ) -> Result<FacetSearchResponse>;
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

pub trait Indexes {
    /// GET /indexes
    fn list_indexes(&self, query: &PaginationQuery) -> Result<IndexList>;

    /// GET /indexes/{indexUid}
    fn get_index(&self, index_uid: &str) -> Result<Index>;

    /// POST /indexes
    fn create_index(&self, request: &CreateIndexRequest) -> Result<TaskInfo>;

    /// PATCH /indexes/{indexUid}
    fn update_index(
        &self,
        index_uid: &str,
        request: &UpdateIndexRequest,
    ) -> Result<TaskInfo>;

    /// POST /swap-indexes
    fn swap_indexes(&self, swaps: &[SwapIndexesRequest]) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}
    fn delete_index(&self, index_uid: &str) -> Result<TaskInfo>;
}

// ─── Tasks ───────────────────────────────────────────────────────────────────

pub trait Tasks {
    /// GET /tasks/{taskUid}
    fn get_task(&self, task_uid: u64) -> Result<Task>;

    /// GET /tasks
    fn list_tasks(&self, filter: &TaskFilter) -> Result<TaskList>;

    /// POST /tasks/cancel
    fn cancel_tasks(&self, filter: &TaskFilter) -> Result<TaskInfo>;

    /// DELETE /tasks
    fn delete_tasks(&self, filter: &TaskFilter) -> Result<TaskInfo>;
}

// ─── Batches ─────────────────────────────────────────────────────────────────

pub trait Batches {
    /// GET /batches/{batchUid}
    fn get_batch(&self, batch_uid: u64) -> Result<Batch>;

    /// GET /batches
    fn list_batches(&self, filter: &TaskFilter) -> Result<BatchList>;
}

// ─── Settings ────────────────────────────────────────────────────────────────

/// Per-index settings management.
///
/// This trait is intentionally wide (18 setting types x 3 operations = 54
/// methods) because it mirrors the Meilisearch REST API 1:1. Each setting
/// type has get/update/reset methods that map directly to their HTTP
/// counterparts. Splitting into sub-traits would break the 1:1 API mapping
/// and complicate the `Engine` implementation without reducing total code.
pub trait SettingsApi {
    /// GET /indexes/{indexUid}/settings
    fn get_settings(&self, index_uid: &str) -> Result<Settings>;

    /// PATCH /indexes/{indexUid}/settings
    fn update_settings(
        &self,
        index_uid: &str,
        settings: &Settings,
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/settings
    fn reset_settings(&self, index_uid: &str) -> Result<TaskInfo>;

    // ── Sub-settings: each follows get / update / reset ──────────────────
    //
    // Default implementations delegate to get_settings / update_settings /
    // reset_settings so that alternative backends only need to implement
    // the three core methods above.
    //
    // **Performance note:** These defaults perform a full settings roundtrip
    // (serialize all settings, update, deserialize) for each sub-setting
    // operation. The `Engine` implementation overrides every method with
    // direct per-setting calls for O(1) performance. Alternative backends
    // using the defaults will have O(N) overhead proportional to the total
    // number of settings per operation. Override individual methods if this
    // becomes a bottleneck.

    fn get_ranking_rules(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.ranking_rules.unwrap_or_default())
    }
    fn update_ranking_rules(&self, index_uid: &str, rules: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.ranking_rules = Some(rules.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_ranking_rules(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.ranking_rules = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_distinct_attribute(&self, index_uid: &str) -> Result<Option<String>> {
        Ok(self.get_settings(index_uid)?.distinct_attribute)
    }
    fn update_distinct_attribute(&self, index_uid: &str, attr: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.distinct_attribute = Some(attr.to_string());
        self.update_settings(index_uid, &s)
    }
    fn reset_distinct_attribute(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.distinct_attribute = None;
        self.update_settings(index_uid, &s)
    }

    fn get_searchable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.searchable_attributes.unwrap_or_default())
    }
    fn update_searchable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.searchable_attributes = Some(attrs.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_searchable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.searchable_attributes = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_displayed_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.displayed_attributes.unwrap_or_default())
    }
    fn update_displayed_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.displayed_attributes = Some(attrs.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_displayed_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.displayed_attributes = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_synonyms(&self, index_uid: &str) -> Result<std::collections::HashMap<String, Vec<String>>> {
        Ok(self.get_settings(index_uid)?.synonyms.unwrap_or_default())
    }
    fn update_synonyms(&self, index_uid: &str, synonyms: &std::collections::HashMap<String, Vec<String>>) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.synonyms = Some(synonyms.clone());
        self.update_settings(index_uid, &s)
    }
    fn reset_synonyms(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.synonyms = Some(std::collections::HashMap::new());
        self.update_settings(index_uid, &s)
    }

    fn get_stop_words(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.stop_words.unwrap_or_default())
    }
    fn update_stop_words(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.stop_words = Some(words.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_stop_words(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.stop_words = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_filterable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.filterable_attributes.unwrap_or_default())
    }
    fn update_filterable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.filterable_attributes = Some(attrs.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_filterable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.filterable_attributes = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_sortable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.sortable_attributes.unwrap_or_default())
    }
    fn update_sortable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.sortable_attributes = Some(attrs.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_sortable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.sortable_attributes = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_typo_tolerance(&self, index_uid: &str) -> Result<TypoTolerance> {
        Ok(self.get_settings(index_uid)?.typo_tolerance.unwrap_or_default())
    }
    fn update_typo_tolerance(&self, index_uid: &str, config: &TypoTolerance) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.typo_tolerance = Some(config.clone());
        self.update_settings(index_uid, &s)
    }
    fn reset_typo_tolerance(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.typo_tolerance = Some(TypoTolerance::default());
        self.update_settings(index_uid, &s)
    }

    fn get_pagination(&self, index_uid: &str) -> Result<Pagination> {
        Ok(self.get_settings(index_uid)?.pagination.unwrap_or_default())
    }
    fn update_pagination(&self, index_uid: &str, config: &Pagination) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.pagination = Some(config.clone());
        self.update_settings(index_uid, &s)
    }
    fn reset_pagination(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.pagination = Some(Pagination::default());
        self.update_settings(index_uid, &s)
    }

    fn get_faceting(&self, index_uid: &str) -> Result<Faceting> {
        Ok(self.get_settings(index_uid)?.faceting.unwrap_or_default())
    }
    fn update_faceting(&self, index_uid: &str, config: &Faceting) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.faceting = Some(config.clone());
        self.update_settings(index_uid, &s)
    }
    fn reset_faceting(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.faceting = Some(Faceting::default());
        self.update_settings(index_uid, &s)
    }

    fn get_dictionary(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.dictionary.unwrap_or_default())
    }
    fn update_dictionary(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.dictionary = Some(words.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_dictionary(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.dictionary = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.separator_tokens.unwrap_or_default())
    }
    fn update_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.separator_tokens = Some(tokens.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.separator_tokens = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_non_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        Ok(self.get_settings(index_uid)?.non_separator_tokens.unwrap_or_default())
    }
    fn update_non_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.non_separator_tokens = Some(tokens.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_non_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.non_separator_tokens = Some(vec![]);
        self.update_settings(index_uid, &s)
    }

    fn get_proximity_precision(&self, index_uid: &str) -> Result<ProximityPrecision> {
        Ok(self.get_settings(index_uid)?.proximity_precision.unwrap_or(ProximityPrecision::ByWord))
    }
    fn update_proximity_precision(&self, index_uid: &str, precision: ProximityPrecision) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.proximity_precision = Some(precision);
        self.update_settings(index_uid, &s)
    }
    fn reset_proximity_precision(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.proximity_precision = None;
        self.update_settings(index_uid, &s)
    }

    fn get_facet_search(&self, index_uid: &str) -> Result<bool> {
        Ok(self.get_settings(index_uid)?.facet_search.unwrap_or(true))
    }
    fn update_facet_search(&self, index_uid: &str, enabled: bool) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.facet_search = Some(enabled);
        self.update_settings(index_uid, &s)
    }
    fn reset_facet_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.facet_search = None;
        self.update_settings(index_uid, &s)
    }

    fn get_prefix_search(&self, index_uid: &str) -> Result<PrefixSearch> {
        Ok(self.get_settings(index_uid)?.prefix_search.unwrap_or(PrefixSearch::IndexingTime))
    }
    fn update_prefix_search(&self, index_uid: &str, mode: PrefixSearch) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.prefix_search = Some(mode);
        self.update_settings(index_uid, &s)
    }
    fn reset_prefix_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.prefix_search = None;
        self.update_settings(index_uid, &s)
    }

    fn get_search_cutoff_ms(&self, index_uid: &str) -> Result<Option<u64>> {
        Ok(self.get_settings(index_uid)?.search_cutoff_ms)
    }
    fn update_search_cutoff_ms(&self, index_uid: &str, ms: u64) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.search_cutoff_ms = Some(ms);
        self.update_settings(index_uid, &s)
    }
    fn reset_search_cutoff_ms(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.search_cutoff_ms = None;
        self.update_settings(index_uid, &s)
    }

    fn get_localized_attributes(&self, index_uid: &str) -> Result<Option<Vec<LocalizedAttribute>>> {
        Ok(self.get_settings(index_uid)?.localized_attributes)
    }
    fn update_localized_attributes(&self, index_uid: &str, attrs: &[LocalizedAttribute]) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.localized_attributes = Some(attrs.to_vec());
        self.update_settings(index_uid, &s)
    }
    fn reset_localized_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.localized_attributes = None;
        self.update_settings(index_uid, &s)
    }

    fn get_embedders(&self, index_uid: &str) -> Result<Option<std::collections::HashMap<String, EmbedderConfig>>> {
        Ok(self.get_settings(index_uid)?.embedders)
    }
    fn update_embedders(&self, index_uid: &str, embedders: &std::collections::HashMap<String, EmbedderConfig>) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.embedders = Some(embedders.clone());
        self.update_settings(index_uid, &s)
    }
    fn reset_embedders(&self, index_uid: &str) -> Result<TaskInfo> {
        let mut s = Settings::default();
        s.embedders = None;
        self.update_settings(index_uid, &s)
    }
}

// ─── Keys ────────────────────────────────────────────────────────────────────

pub trait Keys {
    /// GET /keys
    fn list_keys(&self, query: &PaginationQuery) -> Result<ApiKeyList>;

    /// GET /keys/{keyId}
    fn get_key(&self, key_id: &str) -> Result<ApiKey>;

    /// POST /keys
    fn create_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKey>;

    /// PATCH /keys/{keyId}
    fn update_key(&self, key_id: &str, request: &UpdateApiKeyRequest) -> Result<ApiKey>;

    /// DELETE /keys/{keyId}
    fn delete_key(&self, key_id: &str) -> Result<()>;
}

// ─── Webhooks ────────────────────────────────────────────────────────────────

pub trait Webhooks {
    /// GET /webhooks
    fn list_webhooks(&self) -> Result<Vec<Webhook>>;

    /// GET /webhooks/{uuid}
    fn get_webhook(&self, webhook_uid: &str) -> Result<Webhook>;

    /// POST /webhooks
    fn create_webhook(&self, request: &CreateWebhookRequest) -> Result<Webhook>;

    /// PATCH /webhooks/{uuid}
    fn update_webhook(
        &self,
        webhook_uid: &str,
        request: &UpdateWebhookRequest,
    ) -> Result<Webhook>;

    /// DELETE /webhooks/{uuid}
    fn delete_webhook(&self, webhook_uid: &str) -> Result<()>;
}

// ─── Stats & system ──────────────────────────────────────────────────────────

pub trait System {
    /// GET /stats
    fn global_stats(&self) -> Result<GlobalStats>;

    /// GET /indexes/{indexUid}/stats
    fn index_stats(&self, index_uid: &str) -> Result<IndexStats>;

    /// GET /version
    fn version(&self) -> Result<Version>;

    /// GET /health
    fn health(&self) -> Result<Health>;

    /// POST /dumps
    fn create_dump(&self) -> Result<TaskInfo>;

    /// POST /snapshots
    fn create_snapshot(&self) -> Result<TaskInfo>;

    /// POST /export
    fn export(&self, request: &ExportRequest) -> Result<TaskInfo>;
}

// ─── Experimental features ───────────────────────────────────────────────────

pub trait ExperimentalFeaturesApi {
    /// GET /experimental-features
    fn get_experimental_features(&self) -> Result<ExperimentalFeatures>;

    /// PATCH /experimental-features
    fn update_experimental_features(
        &self,
        features: &ExperimentalFeatures,
    ) -> Result<ExperimentalFeatures>;
}

// ─── Composite trait ─────────────────────────────────────────────────────────

/// Full Meilisearch API surface. Implement this or compose from the individual traits.
pub trait MeilisearchApi:
    Documents + Search + Indexes + Tasks + Batches + SettingsApi + Keys + Webhooks + System + ExperimentalFeaturesApi
{
}

impl<T> MeilisearchApi for T where
    T: Documents + Search + Indexes + Tasks + Batches + SettingsApi + Keys + Webhooks + System + ExperimentalFeaturesApi
{
}
