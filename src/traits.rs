use serde_json::Value;

use crate::types::*;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ─── Documents ───────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Documents {
    /// GET /indexes/{indexUid}/documents/{documentId}
    async fn get_document(
        &self,
        index_uid: &str,
        document_id: &str,
        query: &DocumentQuery,
    ) -> Result<Value>;

    /// GET /indexes/{indexUid}/documents
    async fn get_documents(
        &self,
        index_uid: &str,
        query: &DocumentsQuery,
    ) -> Result<DocumentsResponse>;

    /// POST /indexes/{indexUid}/documents/fetch
    async fn fetch_documents(
        &self,
        index_uid: &str,
        request: &FetchDocumentsRequest,
    ) -> Result<DocumentsResponse>;

    /// POST /indexes/{indexUid}/documents
    async fn add_or_replace_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo>;

    /// PUT /indexes/{indexUid}/documents
    async fn add_or_update_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/documents/{documentId}
    async fn delete_document(&self, index_uid: &str, document_id: &str) -> Result<TaskInfo>;

    /// POST /indexes/{indexUid}/documents/delete
    async fn delete_documents_by_filter(
        &self,
        index_uid: &str,
        request: &DeleteDocumentsByFilterRequest,
    ) -> Result<TaskInfo>;

    /// POST /indexes/{indexUid}/documents/delete-batch
    async fn delete_documents_by_batch(
        &self,
        index_uid: &str,
        document_ids: &[Value],
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/documents
    async fn delete_all_documents(&self, index_uid: &str) -> Result<TaskInfo>;
}

// ─── Search ──────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Search {
    /// POST /indexes/{indexUid}/search
    async fn search(
        &self,
        index_uid: &str,
        request: &SearchRequest,
    ) -> Result<SearchResponse>;

    /// POST /indexes/{indexUid}/similar
    async fn similar(
        &self,
        index_uid: &str,
        request: &SimilarRequest,
    ) -> Result<SimilarResponse>;

    /// POST /multi-search
    async fn multi_search(&self, request: &MultiSearchRequest) -> Result<MultiSearchResponse>;

    /// POST /indexes/{indexUid}/facet-search
    async fn facet_search(
        &self,
        index_uid: &str,
        request: &FacetSearchRequest,
    ) -> Result<FacetSearchResponse>;
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Indexes {
    /// GET /indexes
    async fn list_indexes(&self, query: &PaginationQuery) -> Result<IndexList>;

    /// GET /indexes/{indexUid}
    async fn get_index(&self, index_uid: &str) -> Result<Index>;

    /// POST /indexes
    async fn create_index(&self, request: &CreateIndexRequest) -> Result<TaskInfo>;

    /// PATCH /indexes/{indexUid}
    async fn update_index(
        &self,
        index_uid: &str,
        request: &UpdateIndexRequest,
    ) -> Result<TaskInfo>;

    /// POST /swap-indexes
    async fn swap_indexes(&self, swaps: &[SwapIndexesRequest]) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}
    async fn delete_index(&self, index_uid: &str) -> Result<TaskInfo>;
}

// ─── Tasks ───────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Tasks {
    /// GET /tasks/{taskUid}
    async fn get_task(&self, task_uid: u64) -> Result<Task>;

    /// GET /tasks
    async fn list_tasks(&self, filter: &TaskFilter) -> Result<TaskList>;

    /// POST /tasks/cancel
    async fn cancel_tasks(&self, filter: &TaskFilter) -> Result<TaskInfo>;

    /// DELETE /tasks
    async fn delete_tasks(&self, filter: &TaskFilter) -> Result<TaskInfo>;
}

// ─── Batches ─────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Batches {
    /// GET /batches/{batchUid}
    async fn get_batch(&self, batch_uid: u64) -> Result<Batch>;

    /// GET /batches
    async fn list_batches(&self, filter: &TaskFilter) -> Result<BatchList>;
}

// ─── Settings ────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait SettingsApi {
    /// GET /indexes/{indexUid}/settings
    async fn get_settings(&self, index_uid: &str) -> Result<Settings>;

    /// PATCH /indexes/{indexUid}/settings
    async fn update_settings(
        &self,
        index_uid: &str,
        settings: &Settings,
    ) -> Result<TaskInfo>;

    /// DELETE /indexes/{indexUid}/settings
    async fn reset_settings(&self, index_uid: &str) -> Result<TaskInfo>;

    // ── Sub-settings: each follows get / update / reset ──────────────────

    async fn get_ranking_rules(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_ranking_rules(&self, index_uid: &str, rules: &[String]) -> Result<TaskInfo>;
    async fn reset_ranking_rules(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_distinct_attribute(&self, index_uid: &str) -> Result<Option<String>>;
    async fn update_distinct_attribute(&self, index_uid: &str, attr: &str) -> Result<TaskInfo>;
    async fn reset_distinct_attribute(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_searchable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_searchable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    async fn reset_searchable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_displayed_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_displayed_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    async fn reset_displayed_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_synonyms(&self, index_uid: &str) -> Result<std::collections::HashMap<String, Vec<String>>>;
    async fn update_synonyms(&self, index_uid: &str, synonyms: &std::collections::HashMap<String, Vec<String>>) -> Result<TaskInfo>;
    async fn reset_synonyms(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_stop_words(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_stop_words(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo>;
    async fn reset_stop_words(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_filterable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_filterable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    async fn reset_filterable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_sortable_attributes(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_sortable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo>;
    async fn reset_sortable_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_typo_tolerance(&self, index_uid: &str) -> Result<TypoTolerance>;
    async fn update_typo_tolerance(&self, index_uid: &str, config: &TypoTolerance) -> Result<TaskInfo>;
    async fn reset_typo_tolerance(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_pagination(&self, index_uid: &str) -> Result<Pagination>;
    async fn update_pagination(&self, index_uid: &str, config: &Pagination) -> Result<TaskInfo>;
    async fn reset_pagination(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_faceting(&self, index_uid: &str) -> Result<Faceting>;
    async fn update_faceting(&self, index_uid: &str, config: &Faceting) -> Result<TaskInfo>;
    async fn reset_faceting(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_dictionary(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_dictionary(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo>;
    async fn reset_dictionary(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo>;
    async fn reset_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_non_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>>;
    async fn update_non_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo>;
    async fn reset_non_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_proximity_precision(&self, index_uid: &str) -> Result<String>;
    async fn update_proximity_precision(&self, index_uid: &str, precision: &str) -> Result<TaskInfo>;
    async fn reset_proximity_precision(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_facet_search(&self, index_uid: &str) -> Result<bool>;
    async fn update_facet_search(&self, index_uid: &str, enabled: bool) -> Result<TaskInfo>;
    async fn reset_facet_search(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_prefix_search(&self, index_uid: &str) -> Result<String>;
    async fn update_prefix_search(&self, index_uid: &str, mode: &str) -> Result<TaskInfo>;
    async fn reset_prefix_search(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_search_cutoff_ms(&self, index_uid: &str) -> Result<Option<u64>>;
    async fn update_search_cutoff_ms(&self, index_uid: &str, ms: u64) -> Result<TaskInfo>;
    async fn reset_search_cutoff_ms(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_localized_attributes(&self, index_uid: &str) -> Result<Option<Vec<LocalizedAttribute>>>;
    async fn update_localized_attributes(&self, index_uid: &str, attrs: &[LocalizedAttribute]) -> Result<TaskInfo>;
    async fn reset_localized_attributes(&self, index_uid: &str) -> Result<TaskInfo>;

    async fn get_embedders(&self, index_uid: &str) -> Result<Option<std::collections::HashMap<String, EmbedderConfig>>>;
    async fn update_embedders(&self, index_uid: &str, embedders: &std::collections::HashMap<String, EmbedderConfig>) -> Result<TaskInfo>;
    async fn reset_embedders(&self, index_uid: &str) -> Result<TaskInfo>;
}

// ─── Keys ────────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Keys {
    /// GET /keys
    async fn list_keys(&self, query: &PaginationQuery) -> Result<ApiKeyList>;

    /// GET /keys/{keyId}
    async fn get_key(&self, key_id: &str) -> Result<ApiKey>;

    /// POST /keys
    async fn create_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKey>;

    /// PATCH /keys/{keyId}
    async fn update_key(&self, key_id: &str, request: &UpdateApiKeyRequest) -> Result<ApiKey>;

    /// DELETE /keys/{keyId}
    async fn delete_key(&self, key_id: &str) -> Result<()>;
}

// ─── Webhooks ────────────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait Webhooks {
    /// GET /webhooks
    async fn list_webhooks(&self) -> Result<Vec<Webhook>>;

    /// GET /webhooks/{uuid}
    async fn get_webhook(&self, webhook_uid: &str) -> Result<Webhook>;

    /// POST /webhooks
    async fn create_webhook(&self, request: &CreateWebhookRequest) -> Result<Webhook>;

    /// PATCH /webhooks/{uuid}
    async fn update_webhook(
        &self,
        webhook_uid: &str,
        request: &UpdateWebhookRequest,
    ) -> Result<Webhook>;

    /// DELETE /webhooks/{uuid}
    async fn delete_webhook(&self, webhook_uid: &str) -> Result<()>;
}

// ─── Stats & system ──────────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait System {
    /// GET /stats
    async fn global_stats(&self) -> Result<GlobalStats>;

    /// GET /indexes/{indexUid}/stats
    async fn index_stats(&self, index_uid: &str) -> Result<IndexStats>;

    /// GET /version
    async fn version(&self) -> Result<Version>;

    /// GET /health
    async fn health(&self) -> Result<Health>;

    /// POST /dumps
    async fn create_dump(&self) -> Result<TaskInfo>;

    /// POST /snapshots
    async fn create_snapshot(&self) -> Result<TaskInfo>;

    /// POST /export
    async fn export(&self, request: &ExportRequest) -> Result<TaskInfo>;
}

// ─── Experimental features ───────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait ExperimentalFeaturesApi {
    /// GET /experimental-features
    async fn get_experimental_features(&self) -> Result<ExperimentalFeatures>;

    /// PATCH /experimental-features
    async fn update_experimental_features(
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
