//! SurrealDB-backed vector store implementation.
//!
//! This module provides a [`VectorStore`] implementation using SurrealDB's
//! native HNSW vector index for approximate nearest neighbor search.
//!
//! # Features
//!
//! - HNSW index with configurable parameters (M, EF)
//! - Cosine distance metric
//! - Optional binary quantization
//! - Hybrid search with BM25 full-text search and RRF fusion
//!
//! # Example
//!
//! ```no_run
//! use wilysearch::core::vector::surrealdb::{SurrealDbVectorStore, SurrealDbVectorStoreConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = SurrealDbVectorStoreConfig {
//!     connection_string: "memory".to_string(),
//!     namespace: "test".to_string(),
//!     database: "vectors".to_string(),
//!     table: "embeddings".to_string(),
//!     dimensions: 384,
//!     ..Default::default()
//! };
//!
//! let store = SurrealDbVectorStore::new(config).await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use anyhow::Context;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::{connect, Any};
use surrealdb::Surreal;
use tokio::runtime::Runtime;

use super::VectorStore;
use crate::core::error::{Error as CoreError, Result as CoreResult};

/// Configuration for a SurrealDB vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SurrealDbVectorStoreConfig {
    /// Connection string for SurrealDB.
    ///
    /// Examples:
    /// - `"memory"` - In-memory database
    /// - `"file:///path/to/db"` - File-based RocksDB storage
    /// - `"ws://localhost:8000"` - WebSocket connection to remote server
    pub connection_string: String,

    /// SurrealDB namespace.
    pub namespace: String,

    /// SurrealDB database name.
    pub database: String,

    /// Table name for storing vectors.
    pub table: String,

    /// Vector dimensions.
    pub dimensions: usize,

    /// HNSW M parameter (max connections per node).
    /// Higher values improve recall but increase memory usage.
    /// Default: 16
    pub hnsw_m: usize,

    /// HNSW EF construction parameter.
    /// Higher values improve index quality but slow down insertions.
    /// Default: 500
    pub hnsw_ef: usize,

    /// Whether to use binary quantization.
    /// Reduces memory usage at the cost of some accuracy.
    /// Default: false
    pub quantized: bool,

    /// Optional authentication credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<SurrealDbAuth>,
}

/// Authentication credentials for SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurrealDbAuth {
    pub username: String,
    pub password: String,
}

impl Default for SurrealDbVectorStoreConfig {
    fn default() -> Self {
        Self {
            connection_string: "memory".to_string(),
            namespace: "meilisearch".to_string(),
            database: "vectors".to_string(),
            table: "embeddings".to_string(),
            dimensions: 384,
            hnsw_m: 16,
            hnsw_ef: 500,
            quantized: false,
            auth: None,
        }
    }
}

/// A vector document stored in SurrealDB.
#[derive(Debug, Serialize, Deserialize)]
struct VectorDocument {
    doc_id: u32,
    embedding: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_content: Option<String>,
}

/// Result from a vector search query.
#[derive(Debug, Deserialize)]
struct SearchResult {
    doc_id: u32,
    #[serde(default)]
    distance: f32,
}

/// Result from a hybrid search query.
#[derive(Debug, Deserialize)]
struct HybridSearchResult {
    doc_id: u32,
    #[serde(default)]
    score: f32,
}

/// SurrealDB-backed vector store.
///
/// This implementation uses SurrealDB's native HNSW index for efficient
/// approximate nearest neighbor search with cosine distance.
pub struct SurrealDbVectorStore {
    db: Arc<Surreal<Any>>,
    config: SurrealDbVectorStoreConfig,
    runtime: Arc<Runtime>,
}

/// Validate that a SurrealDB identifier (table, namespace, database) contains
/// only alphanumeric characters and underscores. This prevents query injection
/// via format!-interpolated identifiers in SurrealQL strings.
fn validate_identifier(name: &str, label: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        anyhow::bail!(
            "{label} '{}' contains invalid characters; only alphanumeric and underscores are allowed",
            name
        );
    }
    Ok(())
}

impl SurrealDbVectorStore {
    /// Create a new SurrealDB vector store with an internal runtime.
    ///
    /// This will:
    /// 1. Validate identifier names (table, namespace, database)
    /// 2. Connect to the SurrealDB instance
    /// 3. Select the namespace and database
    /// 4. Create/verify the table schema and HNSW index
    ///
    /// Internally creates a `current_thread` tokio runtime for bridging the
    /// sync [`VectorStore`] trait methods to async SurrealDB calls. When the
    /// sync trait methods are called from outside any tokio runtime,
    /// [`block_on`](Self::block_on) uses this internal runtime directly. When
    /// called from within an existing **multi-thread** runtime, `block_on`
    /// uses `block_in_place` together with the internal runtime. Calling from
    /// a `current_thread` runtime returns an error.
    ///
    /// If you already have a multi-thread tokio runtime, prefer
    /// [`with_runtime`](Self::with_runtime) to share it instead.
    pub async fn new(config: SurrealDbVectorStoreConfig) -> anyhow::Result<Self> {
        validate_identifier(&config.table, "table")?;
        validate_identifier(&config.namespace, "namespace")?;
        validate_identifier(&config.database, "database")?;

        let db = connect(&config.connection_string)
            .await
            .context("Failed to connect to SurrealDB")?;

        // Authenticate if credentials provided
        if let Some(auth) = &config.auth {
            db.signin(surrealdb::opt::auth::Root {
                username: &auth.username,
                password: &auth.password,
            })
            .await
            .context("Failed to authenticate with SurrealDB")?;
        }

        // Select namespace and database
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .context("Failed to select namespace/database")?;

        let store = Self {
            db: Arc::new(db),
            config,
            runtime: Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("Failed to create tokio runtime")?,
            ),
        };

        // Initialize schema
        store.init_schema().await?;

        Ok(store)
    }

    /// Create a new SurrealDB vector store with a shared runtime.
    ///
    /// Use this instead of [`new`](Self::new) when you already have a
    /// multi-thread tokio runtime and want the sync [`VectorStore`] trait
    /// methods to reuse it for `block_in_place` calls. This avoids creating
    /// a redundant internal runtime and keeps all async work on one executor.
    ///
    /// The supplied runtime **must** be a multi-thread runtime; passing a
    /// `current_thread` runtime will cause [`block_on`](Self::block_on) to
    /// return an error when invoked from within that runtime.
    pub async fn with_runtime(
        config: SurrealDbVectorStoreConfig,
        runtime: Arc<Runtime>,
    ) -> anyhow::Result<Self> {
        validate_identifier(&config.table, "table")?;
        validate_identifier(&config.namespace, "namespace")?;
        validate_identifier(&config.database, "database")?;

        let db = connect(&config.connection_string)
            .await
            .context("Failed to connect to SurrealDB")?;

        if let Some(auth) = &config.auth {
            db.signin(surrealdb::opt::auth::Root {
                username: &auth.username,
                password: &auth.password,
            })
            .await
            .context("Failed to authenticate with SurrealDB")?;
        }

        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .context("Failed to select namespace/database")?;

        let store = Self {
            db: Arc::new(db),
            config,
            runtime,
        };

        store.init_schema().await?;

        Ok(store)
    }

    /// Initialize the database schema.
    async fn init_schema(&self) -> anyhow::Result<()> {
        let table = &self.config.table;
        let dimensions = self.config.dimensions;
        let hnsw_m = self.config.hnsw_m;
        let hnsw_ef = self.config.hnsw_ef;

        // Define the table and fields
        // Quantized uses int (0/1) for binary quantization; standard uses float.
        let type_spec = if self.config.quantized {
            "array<int>"
        } else {
            "array<float>"
        };

        let schema_query = format!(
            r#"
            DEFINE TABLE IF NOT EXISTS {table} SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS doc_id ON {table} TYPE int;
            DEFINE FIELD IF NOT EXISTS embedding ON {table} TYPE {type_spec};
            DEFINE FIELD IF NOT EXISTS text_content ON {table} TYPE option<string>;
            DEFINE INDEX IF NOT EXISTS {table}_vec ON {table} FIELDS embedding
                HNSW DIMENSION {dimensions} DIST COSINE TYPE F32 EFC {hnsw_ef} M {hnsw_m};
            DEFINE INDEX IF NOT EXISTS {table}_doc ON {table} FIELDS doc_id;
            "#
        );

        let mut response = self
            .db
            .query(&schema_query)
            .await
            .context("Failed to define schema")?;

        let errors: Vec<_> = response.take_errors().into_iter().collect();
        if !errors.is_empty() {
            let messages: Vec<String> = errors
                .into_iter()
                .map(|(idx, err)| format!("statement {idx}: {err}"))
                .collect();
            anyhow::bail!("Schema definition failed: {}", messages.join("; "));
        }

        Ok(())
    }

    /// Shared async implementation for upserting document vectors.
    ///
    /// Uses one SurrealDB transaction per document for fault isolation: if one
    /// document fails, others already committed are preserved. For bulk inserts
    /// this has higher overhead than a single transaction, but avoids all-or-nothing
    /// rollback semantics which would be surprising for partial batch failures.
    async fn upsert_documents_inner(
        db: &Surreal<Any>,
        table: &str,
        documents: &[(u32, Vec<Vec<f32>>)],
    ) -> anyhow::Result<()> {
        for (doc_id, vectors) in documents {
            let mut transaction_query = String::from("BEGIN TRANSACTION;\n");

            transaction_query.push_str(&format!(
                "DELETE FROM {table} WHERE doc_id = $doc_id;\n"
            ));

            for (vec_idx, _vector) in vectors.iter().enumerate() {
                let record_id = if vectors.len() == 1 {
                    format!("{}", doc_id)
                } else {
                    format!("{}_{}", doc_id, vec_idx)
                };

                transaction_query.push_str(&format!(
                    "CREATE {table}:`{record_id}` CONTENT {{ doc_id: $doc_id, embedding: $embedding_{vec_idx} }};\n"
                ));
            }

            transaction_query.push_str("COMMIT TRANSACTION;\n");

            let mut query = db.query(&transaction_query).bind(("doc_id", *doc_id));
            for (vec_idx, vector) in vectors.iter().enumerate() {
                query = query.bind((format!("embedding_{vec_idx}"), vector.clone()));
            }
            query.await
                .with_context(|| format!("Failed to upsert vectors for document {}", doc_id))?;
        }

        Ok(())
    }

    /// Add documents with their vectors asynchronously.
    ///
    /// For each document, this first deletes any existing vector records for that
    /// `doc_id`, then inserts the new vectors. This prevents stale records when a
    /// document is re-indexed with fewer vectors than before.
    pub async fn add_documents_async(&self, documents: &[(u32, Vec<Vec<f32>>)]) -> anyhow::Result<()> {
        Self::upsert_documents_inner(&self.db, &self.config.table, documents).await
    }

    /// Remove documents by their IDs asynchronously.
    pub async fn remove_documents_async(&self, ids: &[u32]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let table = &self.config.table;

        // Delete all records matching any of the doc_ids
        let query = format!(
            r#"
            DELETE FROM {table} WHERE doc_id IN $ids;
            "#
        );

        self.db
            .query(&query)
            .bind(("ids", ids.to_vec()))
            .await
            .context("Failed to delete documents")?;

        Ok(())
    }

    /// Perform vector search asynchronously.
    pub async fn search_async(
        &self,
        vector: &[f32],
        limit: usize,
        filter: Option<&RoaringBitmap>,
    ) -> anyhow::Result<Vec<(u32, f32)>> {
        let table = &self.config.table;

        // Convert bitmap once, reuse for both query building and binding
        let allowed_ids: Option<Vec<u32>> = filter.map(|b| b.iter().collect());

        if let Some(ref ids) = allowed_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
        }

        // Build and execute the query
        let mut response = if let Some(ref ids) = allowed_ids {
            let query = format!(
                r#"
                SELECT doc_id, vector::distance::knn() AS distance
                FROM {table}
                WHERE embedding <|{limit},COSINE|> $query_vec
                  AND doc_id IN $allowed_ids
                ORDER BY distance
                LIMIT {limit};
                "#
            );

            self.db
                .query(&query)
                .bind(("query_vec", vector.to_vec()))
                .bind(("allowed_ids", ids.clone()))
                .await
                .context("Failed to execute vector search")?
        } else {
            let query = format!(
                r#"
                SELECT doc_id, vector::distance::knn() AS distance
                FROM {table}
                WHERE embedding <|{limit},COSINE|> $query_vec
                ORDER BY distance
                LIMIT {limit};
                "#
            );

            self.db
                .query(&query)
                .bind(("query_vec", vector.to_vec()))
                .await
                .context("Failed to execute vector search")?
        };

        let results: Vec<SearchResult> = response.take(0usize).context("Failed to parse search results")?;

        // Deduplicate by doc_id (in case multiple vectors per document),
        // converting distance to similarity (1.0 - distance)
        let mut seen = std::collections::HashSet::new();
        let deduplicated: Vec<(u32, f32)> = results
            .into_iter()
            .filter(|r| seen.insert(r.doc_id))
            .map(|r| (r.doc_id, 1.0 - r.distance))
            .collect();

        Ok(deduplicated)
    }

    /// Get the configured dimensions.
    pub fn dimensions_async(&self) -> anyhow::Result<Option<usize>> {
        Ok(Some(self.config.dimensions))
    }

    /// Perform hybrid search combining BM25 full-text search and vector KNN with RRF fusion.
    ///
    /// This requires:
    /// 1. A `text_content` field populated in the documents
    /// 2. A full-text index defined on `text_content`
    ///
    /// # Arguments
    ///
    /// * `text_query` - The text query for BM25 full-text search
    /// * `vector` - The query vector for KNN search
    /// * `limit` - Maximum number of results to return
    /// * `k_constant` - RRF constant (default: 60). Higher values give more weight to top-ranked results.
    ///
    /// # Returns
    ///
    /// A list of (document_id, combined_score) tuples, ordered by relevance.
    pub async fn hybrid_search(
        &self,
        text_query: &str,
        vector: &[f32],
        limit: usize,
        k_constant: usize,
    ) -> anyhow::Result<Vec<(u32, f32)>> {
        let table = &self.config.table;
        let k = k_constant;

        // Hybrid search using SurrealDB's search::rrf function
        // This combines BM25 text search score with vector KNN distance
        let query = format!(
            r#"
            LET $vec_results = (
                SELECT doc_id, vector::distance::knn() AS vec_dist
                FROM {table}
                WHERE embedding <|{limit},COSINE|> $query_vec
                ORDER BY vec_dist
                LIMIT {limit}
            );

            LET $text_results = (
                SELECT doc_id, search::score(0) AS text_score
                FROM {table}
                WHERE text_content @0@ $text_query
                ORDER BY text_score DESC
                LIMIT {limit}
            );

            -- Combine results using RRF
            -- RRF score = sum(1 / (k + rank)) for each ranking
            SELECT
                doc_id,
                (
                    IF $vec_rank != NONE THEN 1.0 / ({k} + $vec_rank) ELSE 0 END +
                    IF $text_rank != NONE THEN 1.0 / ({k} + $text_rank) ELSE 0 END
                ) AS score
            FROM (
                SELECT DISTINCT doc_id FROM array::concat($vec_results.doc_id, $text_results.doc_id)
            )
            LET $vec_rank = array::find_index($vec_results.doc_id, doc_id)
            LET $text_rank = array::find_index($text_results.doc_id, doc_id)
            ORDER BY score DESC
            LIMIT {limit};
            "#
        );

        let mut response = self
            .db
            .query(&query)
            .bind(("query_vec", vector.to_vec()))
            .bind(("text_query", text_query.to_string()))
            .await
            .context("Failed to execute hybrid search")?;

        // The result is from the last statement (index 2 in 0-based)
        let results: Vec<HybridSearchResult> = response
            .take(2usize)
            .context("Failed to parse hybrid search results")?;

        Ok(results.into_iter().map(|r| (r.doc_id, r.score)).collect())
    }

    /// Add a full-text search index on the text_content field.
    ///
    /// Call this before using hybrid_search if you want full-text capabilities.
    pub async fn enable_full_text_search(&self) -> anyhow::Result<()> {
        let table = &self.config.table;

        let query = format!(
            r#"
            DEFINE ANALYZER IF NOT EXISTS meilisearch_analyzer
                TOKENIZERS class, blank
                FILTERS lowercase, ascii, snowball(english);
            DEFINE INDEX IF NOT EXISTS {table}_fts ON {table}
                FIELDS text_content
                FULLTEXT ANALYZER meilisearch_analyzer BM25;
            "#
        );

        self.db
            .query(&query)
            .await
            .context("Failed to enable full-text search")?;

        Ok(())
    }

    /// Update a document's text content for hybrid search.
    pub async fn set_text_content(&self, doc_id: u32, text: &str) -> anyhow::Result<()> {
        let table = &self.config.table;

        let query = format!(
            r#"
            UPDATE {table} SET text_content = $text WHERE doc_id = $doc_id;
            "#
        );

        self.db
            .query(&query)
            .bind(("doc_id", doc_id))
            .bind(("text", text.to_string()))
            .await
            .context("Failed to update text content")?;

        Ok(())
    }

    /// Run an async future that returns `anyhow::Result<T>`, bridging sync
    /// and async calling contexts.
    ///
    /// When called from within a multi-thread tokio runtime, uses
    /// `block_in_place` to avoid nesting runtimes. When called from outside
    /// any runtime, blocks directly on the internal runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if called from within a `current_thread` tokio
    /// runtime, because `block_in_place` is not supported there. If you need
    /// to use `Engine` with SurrealDB from async code, use a multi-thread
    /// runtime or wrap calls in `tokio::task::spawn_blocking`.
    fn block_on<T>(
        &self,
        f: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                    return Err(anyhow::anyhow!(
                        "SurrealDbVectorStore cannot be called from a current_thread tokio \
                         runtime. Use a multi_thread runtime or wrap calls in spawn_blocking."
                    ));
                }
                tokio::task::block_in_place(|| self.runtime.block_on(f))
            }
            Err(_) => self.runtime.block_on(f),
        }
    }

    /// Remove all vectors from the store.
    pub async fn clear_async(&self) -> anyhow::Result<()> {
        let table = &self.config.table;
        let query = format!("DELETE FROM {table};");
        self.db
            .query(&query)
            .await
            .context("Failed to clear vector store")?;
        Ok(())
    }

    /// Get statistics about the vector store.
    pub async fn stats(&self) -> anyhow::Result<VectorStoreStats> {
        let table = &self.config.table;

        let query = format!(
            r#"
            SELECT
                count() AS total_records,
                count(DISTINCT doc_id) AS unique_documents
            FROM {table}
            GROUP ALL;
            "#
        );

        #[derive(Deserialize)]
        struct StatsResult {
            total_records: u64,
            unique_documents: u64,
        }

        let mut response = self.db.query(&query).await.context("Failed to get stats")?;

        let stats: Vec<StatsResult> = response.take(0usize)
            .context("Failed to parse stats result")?;

        let stats_first = stats.into_iter().next();

        Ok(VectorStoreStats {
            total_vectors: stats_first.as_ref().map(|s| s.total_records).unwrap_or(0),
            unique_documents: stats_first.as_ref().map(|s| s.unique_documents).unwrap_or(0),
            dimensions: self.config.dimensions,
        })
    }
}

/// Statistics about the vector store.
#[derive(Debug, Clone)]
pub struct VectorStoreStats {
    /// Total number of vector records.
    pub total_vectors: u64,
    /// Number of unique documents.
    pub unique_documents: u64,
    /// Vector dimensions.
    pub dimensions: usize,
}

/// Convert an `anyhow::Error` into a [`CoreError::VectorStore`] at the trait boundary.
fn to_core_error(e: anyhow::Error) -> CoreError {
    CoreError::VectorStore(e.to_string())
}

// Implement the sync VectorStore trait by wrapping async calls.
//
// Internal helpers return `anyhow::Result`; trait methods convert at the
// boundary via `to_core_error`.
impl VectorStore for SurrealDbVectorStore {
    fn add_documents(&self, documents: &[(u32, Vec<Vec<f32>>)]) -> CoreResult<()> {
        let documents = documents.to_vec();
        let db = Arc::clone(&self.db);
        let table = self.config.table.clone();

        self.block_on(async move {
            Self::upsert_documents_inner(&db, &table, &documents).await
        })
        .map_err(to_core_error)
    }

    fn remove_documents(&self, ids: &[u32]) -> CoreResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let ids = ids.to_vec();
        let db = Arc::clone(&self.db);
        let table = self.config.table.clone();

        self.block_on(async move {
            let query = format!(
                r#"
                DELETE FROM {table} WHERE doc_id IN $ids;
                "#
            );

            db.query(&query)
                .bind(("ids", ids))
                .await
                .context("Failed to delete documents")?;

            Ok(())
        })
        .map_err(to_core_error)
    }

    fn search(
        &self,
        vector: &[f32],
        limit: usize,
        filter: Option<&RoaringBitmap>,
    ) -> CoreResult<Vec<(u32, f32)>> {
        let vector = vector.to_vec();
        let filter_ids: Option<Vec<u32>> = filter.map(|b| b.iter().collect());
        let db = Arc::clone(&self.db);
        let table = self.config.table.clone();

        self.block_on(async move {
            let query = if let Some(ref allowed_ids) = filter_ids {
                if allowed_ids.is_empty() {
                    return Ok(Vec::new());
                }

                format!(
                    r#"
                    SELECT doc_id, vector::distance::knn() AS distance
                    FROM {table}
                    WHERE embedding <|{limit},COSINE|> $query_vec
                      AND doc_id IN $allowed_ids
                    ORDER BY distance
                    LIMIT {limit};
                    "#
                )
            } else {
                format!(
                    r#"
                    SELECT doc_id, vector::distance::knn() AS distance
                    FROM {table}
                    WHERE embedding <|{limit},COSINE|> $query_vec
                    ORDER BY distance
                    LIMIT {limit};
                    "#
                )
            };

            let mut response = if let Some(allowed_ids) = filter_ids {
                db.query(&query)
                    .bind(("query_vec", vector))
                    .bind(("allowed_ids", allowed_ids))
                    .await
                    .context("Failed to execute vector search")?
            } else {
                db.query(&query)
                    .bind(("query_vec", vector))
                    .await
                    .context("Failed to execute vector search")?
            };

            let results: Vec<SearchResult> =
                response.take(0usize).context("Failed to parse search results")?;

            // Deduplicate by doc_id, converting distance to similarity
            let mut seen = std::collections::HashSet::new();
            let deduplicated: Vec<(u32, f32)> = results
                .into_iter()
                .filter(|r| seen.insert(r.doc_id))
                .map(|r| (r.doc_id, 1.0 - r.distance))
                .collect();

            Ok(deduplicated)
        })
        .map_err(to_core_error)
    }

    fn dimensions(&self) -> CoreResult<Option<usize>> {
        Ok(Some(self.config.dimensions))
    }

    fn clear(&self) -> CoreResult<()> {
        let db = Arc::clone(&self.db);
        let table = self.config.table.clone();

        self.block_on(async move {
            let query = format!("DELETE FROM {table};");
            db.query(&query)
                .await
                .context("Failed to clear vector store")?;
            Ok(())
        })
        .map_err(to_core_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_surrealdb_vector_store_basic() {
        let config = SurrealDbVectorStoreConfig {
            connection_string: "memory".to_string(),
            namespace: "test".to_string(),
            database: "vectors".to_string(),
            table: "test_embeddings".to_string(),
            dimensions: 4,
            ..Default::default()
        };

        let store = SurrealDbVectorStore::new(config).await.unwrap();

        // Add some documents
        let docs = vec![
            (1, vec![vec![1.0, 0.0, 0.0, 0.0]]),
            (2, vec![vec![0.0, 1.0, 0.0, 0.0]]),
            (3, vec![vec![0.0, 0.0, 1.0, 0.0]]),
        ];

        store.add_documents_async(&docs).await.unwrap();

        // Search for similar vectors
        let query = vec![1.0, 0.1, 0.0, 0.0];
        let results = store.search_async(&query, 2, None).await.unwrap();

        assert!(!results.is_empty());
        // Document 1 should be closest
        assert_eq!(results[0].0, 1);
    }

    #[tokio::test]
    async fn test_surrealdb_vector_store_with_filter() {
        let config = SurrealDbVectorStoreConfig {
            connection_string: "memory".to_string(),
            namespace: "test".to_string(),
            database: "vectors".to_string(),
            table: "filter_test".to_string(),
            dimensions: 4,
            ..Default::default()
        };

        let store = SurrealDbVectorStore::new(config).await.unwrap();

        let docs = vec![
            (1, vec![vec![1.0, 0.0, 0.0, 0.0]]),
            (2, vec![vec![0.9, 0.1, 0.0, 0.0]]),
            (3, vec![vec![0.0, 1.0, 0.0, 0.0]]),
        ];

        store.add_documents_async(&docs).await.unwrap();

        // Search with filter excluding document 1
        let mut filter = RoaringBitmap::new();
        filter.insert(2);
        filter.insert(3);

        let query = vec![1.0, 0.0, 0.0, 0.0];
        let results = store.search_async(&query, 2, Some(&filter)).await.unwrap();

        assert!(!results.is_empty());
        // Document 2 should be closest among allowed
        assert_eq!(results[0].0, 2);
    }

    #[tokio::test]
    async fn test_surrealdb_vector_store_remove() {
        let config = SurrealDbVectorStoreConfig {
            connection_string: "memory".to_string(),
            namespace: "test".to_string(),
            database: "vectors".to_string(),
            table: "remove_test".to_string(),
            dimensions: 4,
            ..Default::default()
        };

        let store = SurrealDbVectorStore::new(config).await.unwrap();

        let docs = vec![
            (1, vec![vec![1.0, 0.0, 0.0, 0.0]]),
            (2, vec![vec![0.0, 1.0, 0.0, 0.0]]),
        ];

        store.add_documents_async(&docs).await.unwrap();
        store.remove_documents_async(&[1]).await.unwrap();

        let query = vec![1.0, 0.0, 0.0, 0.0];
        let results = store.search_async(&query, 10, None).await.unwrap();

        // Only document 2 should remain
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }

    #[tokio::test]
    async fn test_surrealdb_stats() {
        let config = SurrealDbVectorStoreConfig {
            connection_string: "memory".to_string(),
            namespace: "test".to_string(),
            database: "vectors".to_string(),
            table: "stats_test".to_string(),
            dimensions: 4,
            ..Default::default()
        };

        let store = SurrealDbVectorStore::new(config).await.unwrap();

        let docs = vec![
            (1, vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.5, 0.5, 0.0, 0.0]]),
            (2, vec![vec![0.0, 1.0, 0.0, 0.0]]),
        ];

        store.add_documents_async(&docs).await.unwrap();

        let stats = store.stats().await.unwrap();
        assert_eq!(stats.total_vectors, 3); // 2 vectors for doc 1, 1 for doc 2
        assert_eq!(stats.unique_documents, 2);
        assert_eq!(stats.dimensions, 4);
    }
}
