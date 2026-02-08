use serde_json::Value;

use crate::types::*;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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

    fn get_ranking_rules(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_ranking_rules(&self, index_uid: &str, rules: &[String]) -> Result<TaskInfo>;
    fn reset_ranking_rules(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_distinct_attribute(&self, index_uid: &str) -> Result<Option<String>>;
    fn update_distinct_attribute(&self, index_uid: &str, attr: &str) -> Result<TaskInfo>;
    fn reset_distinct_attribute(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_searchable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_searchable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    fn reset_searchable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_displayed_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_displayed_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    fn reset_displayed_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_synonyms(&self, index_uid: &str) -> Result<std::collections::HashMap<String, Vec<String>>>;
    fn update_synonyms(&self, index_uid: &str, synonyms: &std::collections::HashMap<String, Vec<String>>) -> Result<TaskInfo>;
    fn reset_synonyms(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_stop_words(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_stop_words(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo>;
    fn reset_stop_words(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_filterable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_filterable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    fn reset_filterable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_sortable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_sortable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    fn reset_sortable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_typo_tolerance(&self, index_uid: &str) -> Result<TypoTolerance>;
    fn update_typo_tolerance(&self, index_uid: &str, config: &TypoTolerance) -> Result<TaskInfo>;
    fn reset_typo_tolerance(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_pagination(&self, index_uid: &str) -> Result<Pagination>;
    fn update_pagination(&self, index_uid: &str, config: &Pagination) -> Result<TaskInfo>;
    fn reset_pagination(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_faceting(&self, index_uid: &str) -> Result<Faceting>;
    fn update_faceting(&self, index_uid: &str, config: &Faceting) -> Result<TaskInfo>;
    fn reset_faceting(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_dictionary(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_dictionary(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo>;
    fn reset_dictionary(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo>;
    fn reset_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_non_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>>;
    fn update_non_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo>;
    fn reset_non_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_proximity_precision(&self, index_uid: &str) -> Result<String>;
    fn update_proximity_precision(&self, index_uid: &str, precision: &str) -> Result<TaskInfo>;
    fn reset_proximity_precision(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_facet_search(&self, index_uid: &str) -> Result<bool>;
    fn update_facet_search(&self, index_uid: &str, enabled: bool) -> Result<TaskInfo>;
    fn reset_facet_search(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_prefix_search(&self, index_uid: &str) -> Result<String>;
    fn update_prefix_search(&self, index_uid: &str, mode: &str) -> Result<TaskInfo>;
    fn reset_prefix_search(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_search_cutoff_ms(&self, index_uid: &str) -> Result<Option<u64>>;
    fn update_search_cutoff_ms(&self, index_uid: &str, ms: u64) -> Result<TaskInfo>;
    fn reset_search_cutoff_ms(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_localized_attributes(&self, index_uid: &str) -> Result<Option<Vec<LocalizedAttribute>>>;
    fn update_localized_attributes(&self, index_uid: &str, attrs: &[LocalizedAttribute]) -> Result<TaskInfo>;
    fn reset_localized_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    fn get_embedders(&self, index_uid: &str) -> Result<Option<std::collections::HashMap<String, EmbedderConfig>>>;
    fn update_embedders(&self, index_uid: &str, embedders: &std::collections::HashMap<String, EmbedderConfig>) -> Result<TaskInfo>;
    fn reset_embedders(&self, index_uid: &str) -> Result<TaskInfo>;
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
