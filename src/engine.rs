//! HTTP-less Meilisearch engine implementation.
//!
//! Wraps `crate::core::Meilisearch` and implements all traits from
//! `crate::traits` by delegating directly to the milli search engine.
//! Operations execute synchronously -- no task queue. Mutation methods
//! return a synthetic `TaskInfo` with status `Succeeded`.
//!
//! # Module size
//!
//! This file is intentionally kept as a single module (~1,700 lines) rather
//! than split into `engine/mod.rs` + `engine/settings.rs` + etc. The settings
//! trait implementation (~500 lines) is repetitive boilerplate that could be
//! macro-generated, but the 1:1 correspondence with trait methods makes the
//! current form easy to navigate and grep. Future work may introduce a macro
//! to reduce the per-setting boilerplate.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::traits::{self, Result};
use crate::types::*;

/// Embedded Meilisearch engine backed by milli/LMDB.
///
/// All operations execute immediately. Mutations return a synthetic
/// [`TaskInfo`] with `status: Succeeded` since there is no task queue.
///
/// # Drop behavior
///
/// When `Engine` is dropped, all cached `milli::Index` handles are released.
/// Each `milli::Index` wraps a `heed::Env` which flushes pending writes and
/// closes the LMDB environment in its own `Drop` implementation. No explicit
/// `Drop` impl is needed on `Engine` itself.
pub struct Engine {
    inner: crate::core::Meilisearch,
    /// In-memory task counter; resets to 0 on restart. Task UIDs are not
    /// persisted and may collide across engine restarts. Consumers that need
    /// stable task references should use their own persistent counter.
    task_counter: AtomicU64,
    task_counter_path: std::path::PathBuf,
    dump_dir: std::path::PathBuf,
    snapshot_dir: std::path::PathBuf,
    /// TODO: Used to serialize dump/snapshot operations. Wire into create_dump/create_snapshot.
    dump_lock: std::sync::Mutex<()>,
    // TODO: Apply search defaults (limit, matching strategy) in convert_search_request.
    // TODO: Initialize preprocessing pipeline from this config.
    // TODO: Build RAG pipeline from this config when retrieval is requested.
    #[allow(dead_code)]
    rag_config: crate::config::RagConfig,
}

impl Engine {
    /// Create a new embedded engine with the given options.
    pub fn new(options: crate::core::MeilisearchOptions) -> Result<Self> {
        let dump_dir = options.db_path.join("dumps");
        let snapshot_dir = options.db_path.join("snapshots");
        let task_counter_path = options.db_path.join("task_counter");

        let start_uid = if task_counter_path.exists() {
            std::fs::read_to_string(&task_counter_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        let inner = crate::core::Meilisearch::new(options)?;
        Ok(Self {
            inner,
            task_counter: AtomicU64::new(start_uid),
            task_counter_path,
            dump_dir,
            snapshot_dir,
            dump_lock: std::sync::Mutex::new(()),
            rag_config: crate::config::RagConfig::default(),
        })
    }

    /// Create a new engine with default options.
    pub fn default_engine() -> Result<Self> {
        Self::new(crate::core::MeilisearchOptions::default())
    }

    /// Create a new engine from a [`WilysearchConfig`](crate::config::WilysearchConfig).
    ///
    /// This applies all configuration sections:
    /// - **engine** -- LMDB database path and mmap sizes
    /// - **experimental** -- runtime feature flags
    /// - **vector_store** -- SurrealDB vector store (requires `surrealdb` feature)
    /// - **search_defaults**, **preprocessing**, **rag** -- stored for later use
    pub fn with_config(config: crate::config::WilysearchConfig) -> Result<Self> {
        let options: crate::core::MeilisearchOptions = config.engine.into();
        let dump_dir = options.db_path.join("dumps");
        let snapshot_dir = options.db_path.join("snapshots");
        let task_counter_path = options.db_path.join("task_counter");

        let start_uid = if task_counter_path.exists() {
            std::fs::read_to_string(&task_counter_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        #[allow(unused_mut)]
        let mut inner = crate::core::Meilisearch::new(options)?;

        // Attach SurrealDB vector store if configured (feature-gated).
        #[cfg(feature = "surrealdb")]
        {
            if let Some(vs_config) = config.vector_store {
                let surreal_config: crate::core::vector::surrealdb::SurrealDbVectorStoreConfig =
                    vs_config.into();
                // Use a current_thread runtime for lower overhead as requested in review
                let rt = std::sync::Arc::new(
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| crate::core::error::Error::Internal(e.to_string()))?
                );
                let store = rt
                    .block_on(
                        crate::core::vector::surrealdb::SurrealDbVectorStore::with_runtime(
                            surreal_config,
                            rt.clone(),
                        ),
                    )
                    .map_err(|e| crate::core::error::Error::Internal(e.to_string()))?;
                inner = inner.with_vector_store(std::sync::Arc::new(store));
            }
        }

        // Apply experimental feature flags.
        let exp_features: crate::core::ExperimentalFeatures = config.experimental.into();
        inner.update_experimental_features(exp_features);

        Ok(Self {
            inner,
            task_counter: AtomicU64::new(start_uid),
            task_counter_path,
            dump_dir,
            snapshot_dir,
            dump_lock: std::sync::Mutex::new(()),
            rag_config: config.rag,
        })
    }

    /// Create a new engine from a TOML configuration file.
    ///
    /// Loads the file with [`WilysearchConfig::from_file`](crate::config::WilysearchConfig::from_file),
    /// applying environment variable overrides, then delegates to [`Engine::with_config`].
    pub fn from_config_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let config = crate::config::WilysearchConfig::from_file(path).map_err(|e| {
            crate::core::error::Error::Internal(e.to_string())
        })?;
        Self::with_config(config)
    }

    fn next_task(&self, task_type: &str, index_uid: Option<&str>) -> TaskInfo {
        let uid = self.task_counter.fetch_add(1, Ordering::Relaxed);
        // Non-atomic write: a crash mid-write could truncate this file, causing
        // counter reuse after restart. This is acceptable because task UIDs are
        // ephemeral in embedded mode (no persistent task queue) and collisions
        // only affect synthetic TaskInfo.task_uid values.
        if let Err(e) = std::fs::write(&self.task_counter_path, (uid + 1).to_string()) {
            tracing::warn!(error = %e, "failed to persist task counter");
        }
        TaskInfo {
            task_uid: uid,
            index_uid: index_uid.map(String::from),
            status: TaskStatus::Succeeded,
            r#type: task_type.to_string(),
            enqueued_at: now_iso8601(),
        }
    }

    fn resolve_index(&self, uid: &str) -> Result<std::sync::Arc<crate::core::Index>> {
        Ok(self.inner.get_index(uid)?)
    }

    fn mutation_task(&self, index_uid: &str, task_type: &str) -> Result<TaskInfo> {
        self.inner.touch_index_updated(index_uid)?;
        Ok(self.next_task(task_type, Some(index_uid)))
    }
}

use crate::core::now_iso8601;

// ─── Type conversion helpers ─────────────────────────────────────────────────

/// Saturating cast from `usize` to `u32`.
/// Returns `u32::MAX` when the value exceeds `u32::MAX` instead of silently
/// truncating via `as u32`.
fn saturating_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Infallible cast from `usize` to `u64`.
/// On platforms where `usize` is 64-bit this is a no-op; on 32-bit platforms
/// the value always fits. Included for consistency with `saturating_u32`.
fn usize_to_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// Saturating cast from `u128` to `u64`.
/// Returns `u64::MAX` when the value exceeds `u64::MAX` (e.g. processing
/// times that overflow 64 bits -- astronomically unlikely but handled safely).
fn saturating_u128_to_u64(v: u128) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn convert_search_request(req: &SearchRequest) -> crate::core::SearchQuery {
    let mut q = match &req.q {
        Some(s) => crate::core::SearchQuery::new(s.as_str()),
        None => crate::core::SearchQuery::match_all(),
    };
    if let Some(offset) = req.offset {
        q = q.with_offset(offset as usize);
    }
    if let Some(limit) = req.limit {
        q = q.with_limit(limit as usize);
    }
    if let Some(page) = req.page {
        q = q.with_page(page as usize);
    }
    if let Some(hpp) = req.hits_per_page {
        q = q.with_hits_per_page(hpp as usize);
    }
    if let Some(ref attrs) = req.attributes_to_retrieve {
        q = q.with_attributes_to_retrieve(attrs.iter().cloned());
    }
    if let Some(ref attrs) = req.attributes_to_highlight {
        q = q.with_attributes_to_highlight(attrs.iter().cloned());
    }
    if let Some(ref attrs) = req.attributes_to_crop {
        q = q.with_attributes_to_crop(attrs.clone());
    }
    if let Some(len) = req.crop_length {
        q = q.with_crop_length(len as usize);
    }
    if let Some(ref marker) = req.crop_marker {
        q = q.with_crop_marker(marker.as_str());
    }
    if let Some(ref filter) = req.filter {
        q = q.with_filter(filter.clone());
    }
    if let Some(show) = req.show_matches_position {
        q = q.with_matches_position(show);
    }
    if let Some(ref facets) = req.facets {
        q = q.with_facets(facets.clone());
    }
    if let Some(ref sort) = req.sort {
        q = q.with_sort(sort.clone());
    }
    if let Some(ref pre) = req.highlight_pre_tag {
        q = q.with_highlight_pre_tag(pre.as_str());
    }
    if let Some(ref post) = req.highlight_post_tag {
        q = q.with_highlight_post_tag(post.as_str());
    }
    if let Some(ref ms) = req.matching_strategy {
        let strategy = match ms {
            MatchingStrategy::Last => crate::core::search::MatchingStrategy::Last,
            MatchingStrategy::All => crate::core::search::MatchingStrategy::All,
            MatchingStrategy::Frequency => crate::core::search::MatchingStrategy::Frequency,
        };
        q = q.with_matching_strategy(strategy);
    }
    if let Some(show) = req.show_ranking_score {
        q = q.with_ranking_score(show);
    }
    if let Some(show) = req.show_ranking_score_details {
        q = q.with_ranking_score_details(show);
    }
    if let Some(ref attrs) = req.attributes_to_search_on {
        q = q.with_attributes_to_search_on(attrs.clone());
    }
    if let Some(retrieve) = req.retrieve_vectors {
        q = q.with_retrieve_vectors(retrieve);
    }
    if let Some(threshold) = req.ranking_score_threshold {
        q = q.with_ranking_score_threshold(threshold);
    }
    if let Some(ref distinct) = req.distinct {
        q = q.with_distinct(distinct.as_str());
    }
    if let Some(ref locales) = req.locales {
        q = q.with_locales(locales.clone());
    }
    if let Some(ref vec) = req.vector {
        q = q.with_vector(vec.iter().map(|&x| x as f32).collect());
    }
    if req.hybrid.is_some() {
        tracing::warn!(
            "SearchRequest.hybrid is set but not yet wired through to the core search engine; \
             hybrid search requires the RAG pipeline or direct vector API"
        );
    }
    q
}

fn convert_search_result(r: &crate::core::SearchResult) -> Result<SearchResponse> {
    let (offset, limit, estimated_total_hits, total_hits, total_pages, page, hits_per_page) =
        match &r.hits_info {
            crate::core::search::HitsInfo::OffsetLimit {
                limit,
                offset,
                estimated_total_hits,
            } => (
                Some(saturating_u32(*offset)),
                Some(saturating_u32(*limit)),
                Some(usize_to_u64(*estimated_total_hits)),
                None,
                None,
                None,
                None,
            ),
            crate::core::search::HitsInfo::Pagination {
                hits_per_page,
                page,
                total_pages,
                total_hits,
            } => (
                None,
                None,
                None,
                Some(usize_to_u64(*total_hits)),
                Some(saturating_u32(*total_pages)),
                Some(saturating_u32(*page)),
                Some(saturating_u32(*hits_per_page)),
            ),
        };

    let facet_distribution = r.facet_distribution.as_ref().map(|fd| {
        fd.iter()
            .map(|(k, v)| (k.clone(), v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect()))
            .collect()
    });
    let facet_stats = r
        .facet_stats
        .as_ref()
        .map(|fs| serde_json::to_value(fs))
        .transpose()?;

    Ok(SearchResponse {
        hits: r.hits.iter().map(convert_hit).collect::<Result<Vec<_>>>()?,
        offset,
        limit,
        estimated_total_hits,
        total_hits,
        total_pages,
        page,
        hits_per_page,
        facet_distribution,
        facet_stats,
        processing_time_ms: saturating_u128_to_u64(r.processing_time_ms),
        query: r.query.clone(),
    })
}

fn convert_hit(h: &crate::core::SearchHit) -> Result<Value> {
    // The SearchHit has `document` flattened, plus optional metadata fields.
    // We serialize the whole hit to produce the correct shape.
    Ok(serde_json::to_value(h)?)
}

fn convert_federation_settings(s: &FederationSettings) -> crate::core::search::Federation {
    crate::core::search::Federation {
        limit: s.limit.unwrap_or(20) as usize,
        offset: s.offset.unwrap_or(0) as usize,
        page: s.page.map(|p| p as usize),
        hits_per_page: s.hits_per_page.map(|h| h as usize),
        facets_by_index: s.facets_by_index.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        merge_facets: s.merge_facets.map(|mf| crate::core::search::MergeFacets {
            max_values_per_facet: mf.max_values_per_facet.map(|v| v as usize),
        }),
    }
}

fn convert_federated_result(r: &crate::core::search::FederatedSearchResult) -> Result<FederatedSearchResponse> {
    let (offset, limit, estimated_total_hits, total_hits, total_pages, page, hits_per_page) =
        match &r.hits_info {
            crate::core::search::HitsInfo::OffsetLimit {
                limit,
                offset,
                estimated_total_hits,
            } => (
                Some(saturating_u32(*offset)),
                Some(saturating_u32(*limit)),
                Some(usize_to_u64(*estimated_total_hits)),
                None,
                None,
                None,
                None,
            ),
            crate::core::search::HitsInfo::Pagination {
                hits_per_page,
                page,
                total_pages,
                total_hits,
            } => (
                None,
                None,
                None,
                Some(usize_to_u64(*total_hits)),
                Some(saturating_u32(*total_pages)),
                Some(saturating_u32(*page)),
                Some(saturating_u32(*hits_per_page)),
            ),
        };

    let facet_distribution = r.facet_distribution.as_ref().map(|fd| {
        fd.iter()
            .map(|(k, v)| (k.clone(), v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect()))
            .collect()
    });
    let facet_stats = r
        .facet_stats
        .as_ref()
        .map(|fs| serde_json::to_value(fs))
        .transpose()?;

    let facets_by_index: HashMap<String, Value> = r
        .facets_by_index
        .iter()
        .map(|(k, v)| Ok((k.clone(), serde_json::to_value(v)?)))
        .collect::<Result<HashMap<_, _>>>()?;

    Ok(FederatedSearchResponse {
        hits: r.hits.iter().map(convert_hit).collect::<Result<Vec<_>>>()?,
        processing_time_ms: saturating_u128_to_u64(r.processing_time_ms),
        offset,
        limit,
        estimated_total_hits,
        total_hits,
        total_pages,
        page,
        hits_per_page,
        facet_distribution,
        facet_stats,
        facets_by_index,
        semantic_hit_count: r.semantic_hit_count,
    })
}

fn convert_settings_to_lib(s: &Settings) -> Result<crate::core::Settings> {
    let mut ms = crate::core::Settings::new();
    if let Some(ref rules) = s.ranking_rules {
        ms = ms.with_ranking_rules(rules.clone());
    }
    if let Some(ref attr) = s.distinct_attribute {
        ms = ms.with_distinct_attribute(attr.clone());
    }
    if let Some(ref attrs) = s.searchable_attributes {
        ms = ms.with_searchable_attributes(attrs.clone());
    }
    if let Some(ref attrs) = s.displayed_attributes {
        ms = ms.with_displayed_attributes(attrs.clone());
    }
    if let Some(ref words) = s.stop_words {
        ms = ms.with_stop_words(words.iter().cloned().collect());
    }
    if let Some(ref syns) = s.synonyms {
        ms = ms.with_synonyms(syns.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }
    if let Some(ref attrs) = s.filterable_attributes {
        ms = ms.with_filterable_attributes(attrs.clone());
    }
    if let Some(ref attrs) = s.sortable_attributes {
        ms = ms.with_sortable_attributes(attrs.iter().cloned().collect());
    }
    if let Some(ref typo) = s.typo_tolerance {
        let lib_typo = crate::core::settings::TypoToleranceSettings {
            enabled: typo.enabled,
            min_word_size_for_typos: typo.min_word_size_for_typos.as_ref().map(|m| {
                crate::core::settings::MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                    two_typos: m.two_typos.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                }
            }),
            disable_on_words: typo.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: typo.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: typo.disable_on_numbers,
        };
        ms = ms.with_typo_tolerance(lib_typo);
    }
    if let Some(ref pag) = s.pagination {
        let lib_pag = crate::core::settings::PaginationSettings {
            max_total_hits: pag.max_total_hits.map(|v| v as usize),
        };
        ms = ms.with_pagination(lib_pag);
    }
    if let Some(ref fac) = s.faceting {
        let sort_map = if let Some(ref m) = fac.sort_facet_values_by {
            let mut result = std::collections::BTreeMap::new();
            for (k, v) in m {
                let sort = match v {
                    FacetValuesSort::Alpha => crate::core::settings::FacetValuesSort::Alpha,
                    FacetValuesSort::Count => crate::core::settings::FacetValuesSort::Count,
                };
                result.insert(k.clone(), sort);
            }
            Some(result)
        } else {
            None
        };
        let lib_fac = crate::core::settings::FacetingSettings {
            max_values_per_facet: fac.max_values_per_facet.map(|v| v as usize),
            sort_facet_values_by: sort_map,
        };
        ms = ms.with_faceting(lib_fac);
    }
    if let Some(ref dict) = s.dictionary {
        ms = ms.with_dictionary(dict.iter().cloned().collect());
    }
    if let Some(ref tokens) = s.separator_tokens {
        ms = ms.with_separator_tokens(tokens.iter().cloned().collect());
    }
    if let Some(ref tokens) = s.non_separator_tokens {
        ms = ms.with_non_separator_tokens(tokens.iter().cloned().collect());
    }
    if let Some(ref prec) = s.proximity_precision {
        let lib_prec = match prec {
            ProximityPrecision::ByWord => crate::core::settings::ProximityPrecision::ByWord,
            ProximityPrecision::ByAttribute => crate::core::settings::ProximityPrecision::ByAttribute,
        };
        ms = ms.with_proximity_precision(lib_prec);
    }
    if let Some(enabled) = s.facet_search {
        ms = ms.with_facet_search(enabled);
    }
    if let Some(ref mode) = s.prefix_search {
        let mode_str = match mode {
            PrefixSearch::IndexingTime => "indexingTime".to_string(),
            PrefixSearch::Disabled => "disabled".to_string(),
        };
        ms = ms.with_prefix_search(mode_str);
    }
    if let Some(ms_val) = s.search_cutoff_ms {
        ms = ms.with_search_cutoff_ms(ms_val);
    }
    if let Some(ref rules) = s.localized_attributes {
        let lib_rules: Vec<crate::core::settings::LocalizedAttributeRule> = rules
            .iter()
            .map(|r| crate::core::settings::LocalizedAttributeRule {
                attribute_patterns: r.attribute_patterns.clone(),
                locales: r.locales.clone(),
            })
            .collect();
        ms = ms.with_localized_attributes(lib_rules);
    }
    if let Some(ref embs) = s.embedders {
        let lib_embs: HashMap<String, crate::core::EmbedderSettings> = embs
            .iter()
            .map(|(k, v)| {
                let mut es = crate::core::EmbedderSettings::default();
                let source = parse_embedder_source(&v.source);
                es.source = Some(source);
                es.api_key = v.api_key.clone();
                es.model = v.model.clone();
                es.dimensions = v.dimensions.map(|d| d as usize);
                es.url = v.url.clone();
                (k.clone(), es)
            })
            .collect();
        ms = ms.with_embedders(lib_embs);
    }
    Ok(ms)
}

/// Parse an embedder source string, accepting both camelCase and lowercase variants.
fn parse_embedder_source(s: &str) -> crate::core::EmbedderSource {
    match s {
        "openai" | "openAi" => crate::core::EmbedderSource::OpenAi,
        "huggingface" | "huggingFace" => crate::core::EmbedderSource::HuggingFace,
        "ollama" => crate::core::EmbedderSource::Ollama,
        "userprovided" | "userProvided" => crate::core::EmbedderSource::UserProvided,
        "rest" => crate::core::EmbedderSource::Rest,
        "composite" => crate::core::EmbedderSource::Composite,
        _ => {
            tracing::warn!(
                source = s,
                "unknown embedder source '{}', falling back to default",
                s
            );
            crate::core::EmbedderSource::default()
        }
    }
}

/// Convert an EmbedderSource to its canonical camelCase string.
fn embedder_source_to_str(s: &crate::core::EmbedderSource) -> &'static str {
    match s {
        crate::core::EmbedderSource::OpenAi => "openAi",
        crate::core::EmbedderSource::HuggingFace => "huggingFace",
        crate::core::EmbedderSource::Ollama => "ollama",
        crate::core::EmbedderSource::UserProvided => "userProvided",
        crate::core::EmbedderSource::Rest => "rest",
        crate::core::EmbedderSource::Composite => "composite",
    }
}

fn convert_settings_from_lib(s: &crate::core::Settings) -> Settings {
    Settings {
        ranking_rules: s.ranking_rules.clone(),
        distinct_attribute: s.distinct_attribute.clone(),
        searchable_attributes: s.searchable_attributes.clone(),
        displayed_attributes: s.displayed_attributes.clone(),
        stop_words: s.stop_words.as_ref().map(|w| w.iter().cloned().collect()),
        synonyms: s.synonyms.as_ref().map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        }),
        filterable_attributes: s.filterable_attributes.clone(),
        sortable_attributes: s.sortable_attributes.as_ref().map(|a| a.iter().cloned().collect()),
        typo_tolerance: s.typo_tolerance.as_ref().map(|t| TypoTolerance {
            enabled: t.enabled,
            min_word_size_for_typos: t.min_word_size_for_typos.as_ref().map(|m| {
                MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| v as u32),
                    two_typos: m.two_typos.map(|v| v as u32),
                }
            }),
            disable_on_words: t.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: t.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: t.disable_on_numbers,
        }),
        pagination: s.pagination.as_ref().map(|p| Pagination {
            max_total_hits: p.max_total_hits.map(usize_to_u64),
        }),
        faceting: s.faceting.as_ref().map(|f| Faceting {
            max_values_per_facet: f.max_values_per_facet.map(usize_to_u64),
            sort_facet_values_by: f.sort_facet_values_by.as_ref().map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let s = match v {
                            crate::core::settings::FacetValuesSort::Count => FacetValuesSort::Count,
                            crate::core::settings::FacetValuesSort::Alpha => FacetValuesSort::Alpha,
                        };
                        (k.clone(), s)
                    })
                    .collect()
            }),
        }),
        dictionary: s.dictionary.as_ref().map(|d| d.iter().cloned().collect()),
        separator_tokens: s.separator_tokens.as_ref().map(|t| t.iter().cloned().collect()),
        non_separator_tokens: s.non_separator_tokens.as_ref().map(|t| t.iter().cloned().collect()),
        proximity_precision: s.proximity_precision.as_ref().map(|p| match p {
            crate::core::settings::ProximityPrecision::ByWord => ProximityPrecision::ByWord,
            crate::core::settings::ProximityPrecision::ByAttribute => ProximityPrecision::ByAttribute,
        }),
        facet_search: s.facet_search,
        prefix_search: s.prefix_search.as_ref().map(|s| match s.as_str() {
            "disabled" => PrefixSearch::Disabled,
            _ => PrefixSearch::IndexingTime,
        }),
        search_cutoff_ms: s.search_cutoff_ms,
        localized_attributes: s.localized_attributes.as_ref().map(|rules| {
            rules
                .iter()
                .map(|r| LocalizedAttribute {
                    attribute_patterns: r.attribute_patterns.clone(),
                    locales: r.locales.clone(),
                })
                .collect()
        }),
        embedders: s.embedders.as_ref().map(|embs| {
            embs.iter()
                .map(|(k, v)| {
                    let source = v
                        .source
                        .as_ref()
                        .map(|s| embedder_source_to_str(s).to_string())
                        .unwrap_or_default();
                    (
                        k.clone(),
                        EmbedderConfig {
                            source,
                            api_key: v.api_key.clone(),
                            model: v.model.clone(),
                            dimensions: v.dimensions.map(saturating_u32),
                            url: v.url.clone(),
                            extra: HashMap::new(),
                        },
                    )
                })
                .collect()
        }),
    }
}

// ─── Documents ───────────────────────────────────────────────────────────────

impl traits::Documents for Engine {
    fn get_document(
        &self,
        index_uid: &str,
        document_id: &str,
        query: &DocumentQuery,
    ) -> Result<Value> {
        let idx = self.resolve_index(index_uid)?;
        let fields: Option<Vec<String>> = query
            .fields
            .as_ref()
            .map(|f| f.split(',').map(|s| s.trim().to_string()).collect());
        let doc = idx.get_document_with_fields(document_id, fields.as_deref())?;
        doc.ok_or_else(|| crate::core::error::Error::DocumentNotFound(document_id.to_string()))
    }

    fn get_documents(
        &self,
        index_uid: &str,
        query: &DocumentsQuery,
    ) -> Result<DocumentsResponse> {
        let idx = self.resolve_index(index_uid)?;
        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query.limit.unwrap_or(20) as usize;

        if query.filter.is_some() || query.ids.is_some() || query.sort.is_some() {
            let options = crate::core::search::GetDocumentsOptions {
                offset,
                limit,
                fields: query.fields.as_ref().map(|f| {
                    f.split(',').map(|s| s.trim().to_string()).collect()
                }),
                filter: query.filter.as_ref().map(|f| Value::String(f.clone())),
                ids: query.ids.as_ref().map(|ids_str| {
                    ids_str.split(',').map(|s| s.trim().to_string()).collect()
                }),
                sort: query.sort.as_ref().map(|s| {
                    s.split(',').map(|s| s.trim().to_string()).collect()
                }),
                ..Default::default()
            };
            let result = idx.get_documents_with_options(&options)?;
            return Ok(DocumentsResponse {
                results: result.documents,
                offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
                limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
                total: result.total,
            });
        }

        let result = idx.get_documents(offset, limit)?;
        Ok(DocumentsResponse {
            results: result.documents,
            offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
            limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
            total: result.total,
        })
    }

    fn fetch_documents(
        &self,
        index_uid: &str,
        request: &FetchDocumentsRequest,
    ) -> Result<DocumentsResponse> {
        let idx = self.resolve_index(index_uid)?;
        let options = crate::core::search::GetDocumentsOptions {
            offset: request.offset.unwrap_or(0) as usize,
            limit: request.limit.unwrap_or(20) as usize,
            fields: request.fields.clone(),
            filter: request.filter.as_ref().map(|f| Value::String(f.clone())),
            ..Default::default()
        };
        let result = idx.get_documents_with_options(&options)?;
        Ok(DocumentsResponse {
            results: result.documents,
            offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
            limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
            total: result.total,
        })
    }

    fn add_or_replace_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.add_documents(documents.to_vec(), query.primary_key.as_deref())?;
        self.mutation_task(index_uid, "documentAdditionOrUpdate")
    }

    fn add_or_update_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_documents(documents.to_vec(), query.primary_key.as_deref())?;
        self.mutation_task(index_uid, "documentAdditionOrUpdate")
    }

    fn delete_document(&self, index_uid: &str, document_id: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.delete_document(document_id)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_documents_by_filter(
        &self,
        index_uid: &str,
        request: &DeleteDocumentsByFilterRequest,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.delete_by_filter(&request.filter)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_documents_by_batch(
        &self,
        index_uid: &str,
        document_ids: &[Value],
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let ids: Vec<String> = document_ids
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            })
            .collect();
        idx.delete_documents(ids)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_all_documents(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.clear()?;
        self.mutation_task(index_uid, "documentDeletion")
    }
}

// ─── Search ──────────────────────────────────────────────────────────────────

impl traits::Search for Engine {
    fn search(
        &self,
        index_uid: &str,
        request: &SearchRequest,
    ) -> Result<SearchResponse> {
        let idx = self.resolve_index(index_uid)?;
        let query = convert_search_request(request);
        let result = idx.search(&query)?;
        convert_search_result(&result)
    }

    fn similar(
        &self,
        index_uid: &str,
        request: &SimilarRequest,
    ) -> Result<SimilarResponse> {
        let idx = self.resolve_index(index_uid)?;

        let embedder = request.embedder.clone().unwrap_or("default".to_string());
        let lib_query = crate::core::search::SimilarQuery {
            id: request.id.clone(),
            offset: request.offset.unwrap_or(0) as usize,
            limit: request.limit.unwrap_or(20) as usize,
            filter: request.filter.clone(),
            embedder,
            attributes_to_retrieve: request
                .attributes_to_retrieve
                .as_ref()
                .map(|a| a.iter().cloned().collect()),
            retrieve_vectors: request.retrieve_vectors.unwrap_or(false),
            show_ranking_score: request.show_ranking_score.unwrap_or(false),
            show_ranking_score_details: request.show_ranking_score_details.unwrap_or(false),
            ranking_score_threshold: request.ranking_score_threshold,
        };
        let result = idx.get_similar_documents(&lib_query)?;

        let (offset, limit, estimated) = match &result.hits_info {
            crate::core::search::HitsInfo::OffsetLimit {
                offset,
                limit,
                estimated_total_hits,
            } => (saturating_u32(*offset), saturating_u32(*limit), usize_to_u64(*estimated_total_hits)),
            crate::core::search::HitsInfo::Pagination { total_hits, .. } => {
                (0, 20, usize_to_u64(*total_hits))
            }
        };

        Ok(SimilarResponse {
            hits: result.hits.iter().map(convert_hit).collect::<Result<Vec<_>>>()?,
            offset,
            limit,
            estimated_total_hits: estimated,
            processing_time_ms: saturating_u128_to_u64(result.processing_time_ms),
            id: Value::String(result.id),
        })
    }

    fn multi_search(&self, request: &MultiSearchRequest) -> Result<MultiSearchResult> {
        if let Some(ref federation) = request.federation {
            let core_federation = convert_federation_settings(federation);
            let core_queries: Vec<crate::core::search::FederatedMultiSearchQuery> = request
                .queries
                .iter()
                .map(|mq| {
                    let query = convert_search_request(&mq.search);
                    let federation_options = mq.federation_options.as_ref().map(|fo| {
                        crate::core::search::FederationOptions {
                            weight: fo.weight.unwrap_or(1.0),
                            query_position: fo.query_position.map(|p| p as usize),
                        }
                    });
                    crate::core::search::FederatedMultiSearchQuery {
                        index_uid: mq.index_uid.clone(),
                        query,
                        federation_options,
                    }
                })
                .collect();

            let result = self.inner.multi_search_federated(core_queries, core_federation)?;
            Ok(MultiSearchResult::Federated(convert_federated_result(&result)?))
        } else {
            let mut results = Vec::with_capacity(request.queries.len());
            for mq in &request.queries {
                let idx = self.resolve_index(&mq.index_uid)?;
                let query = convert_search_request(&mq.search);
                let result = idx.search(&query)?;
                let resp = convert_search_result(&result)?;
                results.push(resp);
            }
            Ok(MultiSearchResult::PerIndex(MultiSearchResponse { results }))
        }
    }

    fn facet_search(
        &self,
        index_uid: &str,
        request: &FacetSearchRequest,
    ) -> Result<FacetSearchResponse> {
        let idx = self.resolve_index(index_uid)?;
        let matching = request.matching_strategy.as_ref().map(|ms| match ms {
            MatchingStrategy::Last => crate::core::search::MatchingStrategy::Last,
            MatchingStrategy::All => crate::core::search::MatchingStrategy::All,
            MatchingStrategy::Frequency => crate::core::search::MatchingStrategy::Frequency,
        });
        let lib_query = crate::core::search::FacetSearchQuery {
            facet_name: request.facet_name.clone(),
            facet_query: request.facet_query.clone(),
            q: request.q.clone(),
            filter: request.filter.clone(),
            matching_strategy: matching.unwrap_or(crate::core::search::MatchingStrategy::Last),
            attributes_to_search_on: request.attributes_to_search_on.clone(),
            ranking_score_threshold: None,
            locales: None,
        };
        let result = idx.facet_search(&lib_query)?;
        Ok(FacetSearchResponse {
            facet_hits: result
                .facet_hits
                .into_iter()
                .map(|h| FacetHit {
                    value: h.value,
                    count: h.count,
                })
                .collect(),
            facet_query: result.facet_query,
            processing_time_ms: saturating_u128_to_u64(result.processing_time_ms),
        })
    }
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

impl traits::Indexes for Engine {
    fn list_indexes(&self, query: &PaginationQuery) -> Result<IndexList> {
        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query.limit.unwrap_or(20) as usize;
        let (infos, total) = self.inner.list_indexes_with_pagination(offset, limit)?;
        let results = infos
            .into_iter()
            .map(|info| Index {
                uid: info.uid,
                primary_key: info.primary_key,
                created_at: info.created_at.unwrap_or_default(),
                updated_at: info.updated_at.unwrap_or_default(),
            })
            .collect();
        Ok(IndexList {
            results,
            offset: saturating_u32(offset),
            limit: saturating_u32(limit),
            total: usize_to_u64(total),
        })
    }

    fn get_index(&self, index_uid: &str) -> Result<Index> {
        let idx = self.resolve_index(index_uid)?;
        let pk = idx.primary_key()?;
        let (created_at, updated_at) = self
            .inner
            .get_index_metadata(index_uid)
            .map(|m| (m.created_at, m.updated_at))
            .unwrap_or_default();
        Ok(Index {
            uid: index_uid.to_string(),
            primary_key: pk,
            created_at,
            updated_at,
        })
    }

    fn create_index(&self, request: &CreateIndexRequest) -> Result<TaskInfo> {
        self.inner
            .create_index(&request.uid, request.primary_key.as_deref())?;
        Ok(self.next_task("indexCreation", Some(&request.uid)))
    }

    fn update_index(
        &self,
        index_uid: &str,
        request: &UpdateIndexRequest,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_primary_key(&request.primary_key)?;
        self.mutation_task(index_uid, "indexUpdate")
    }

    fn swap_indexes(&self, swaps: &[SwapIndexesRequest]) -> Result<TaskInfo> {
        let pairs: Vec<(&str, &str)> = swaps
            .iter()
            .map(|s| (s.indexes[0].as_str(), s.indexes[1].as_str()))
            .collect();
        self.inner.swap_indexes(&pairs)?;
        Ok(self.next_task("indexSwap", None))
    }

    fn delete_index(&self, index_uid: &str) -> Result<TaskInfo> {
        self.inner.delete_index(index_uid)?;
        Ok(self.next_task("indexDeletion", Some(index_uid)))
    }
}

// ─── Tasks (stub -- no task queue in embedded mode) ──────────────────────────

impl traits::Tasks for Engine {
    fn get_task(&self, _task_uid: u64) -> Result<Task> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }

    fn list_tasks(&self, _filter: &TaskFilter) -> Result<TaskList> {
        Ok(TaskList {
            results: vec![],
            total: 0,
            limit: 20,
            from: None,
            next: None,
        })
    }

    fn cancel_tasks(&self, _filter: &TaskFilter) -> Result<TaskInfo> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }

    fn delete_tasks(&self, _filter: &TaskFilter) -> Result<TaskInfo> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }
}

// ─── Batches (stub) ──────────────────────────────────────────────────────────

impl traits::Batches for Engine {
    fn get_batch(&self, _batch_uid: u64) -> Result<Batch> {
        Err(crate::core::error::Error::Internal("Batches are not supported in embedded mode".to_string()))
    }

    fn list_batches(&self, _filter: &TaskFilter) -> Result<BatchList> {
        Ok(BatchList {
            results: vec![],
            total: 0,
            limit: 20,
            from: None,
            next: None,
        })
    }
}

// ─── Settings ────────────────────────────────────────────────────────────────

impl traits::SettingsApi for Engine {
    fn get_settings(&self, index_uid: &str) -> Result<Settings> {
        let idx = self.resolve_index(index_uid)?;
        let lib_settings = idx.get_settings()?;
        Ok(convert_settings_from_lib(&lib_settings))
    }

    fn update_settings(&self, index_uid: &str, settings: &Settings) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_settings = convert_settings_to_lib(settings)?;
        idx.update_settings(&lib_settings)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn reset_settings(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_settings()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    // ── Sub-settings ─────────────────────────────────────────────────────

    fn get_ranking_rules(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_ranking_rules()?.unwrap_or_default())
    }
    fn update_ranking_rules(&self, index_uid: &str, rules: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_ranking_rules(rules.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_ranking_rules(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_ranking_rules()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_distinct_attribute(&self, index_uid: &str) -> Result<Option<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_distinct_attribute()?)
    }
    fn update_distinct_attribute(&self, index_uid: &str, attr: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_distinct_attribute(attr.to_string())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_distinct_attribute(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_distinct_attribute()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_searchable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_searchable_attributes()?.unwrap_or_default())
    }
    fn update_searchable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_searchable_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_searchable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_searchable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_displayed_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_displayed_attributes()?.unwrap_or_default())
    }
    fn update_displayed_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_displayed_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_displayed_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_displayed_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_synonyms(&self, index_uid: &str) -> Result<HashMap<String, Vec<String>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_synonyms()?
            .map(|m| m.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_synonyms(
        &self,
        index_uid: &str,
        synonyms: &HashMap<String, Vec<String>>,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let btree: BTreeMap<String, Vec<String>> = synonyms.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        idx.update_synonyms(btree)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_synonyms(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_synonyms()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_stop_words(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_stop_words()?
            .map(|s| s.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_stop_words(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_stop_words(words.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_stop_words(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_stop_words()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_filterable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_filterable_attributes()?.unwrap_or_default())
    }
    fn update_filterable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_filterable_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_filterable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_filterable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_sortable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_sortable_attributes()?
            .map(|s| s.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_sortable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_sortable_attributes(attrs.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_sortable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_sortable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_typo_tolerance(&self, index_uid: &str) -> Result<TypoTolerance> {
        let idx = self.resolve_index(index_uid)?;
        let lib = idx.get_typo_tolerance()?;
        Ok(lib
            .map(|t| TypoTolerance {
                enabled: t.enabled,
                min_word_size_for_typos: t.min_word_size_for_typos.map(|m| MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| v as u32),
                    two_typos: m.two_typos.map(|v| v as u32),
                }),
                disable_on_words: t.disable_on_words.map(|w| w.into_iter().collect()),
                disable_on_attributes: t.disable_on_attributes.map(|a| a.into_iter().collect()),
                disable_on_numbers: t.disable_on_numbers,
            })
            .unwrap_or_default())
    }
    fn update_typo_tolerance(&self, index_uid: &str, config: &TypoTolerance) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_typo = crate::core::settings::TypoToleranceSettings {
            enabled: config.enabled,
            min_word_size_for_typos: config.min_word_size_for_typos.as_ref().map(|m| {
                crate::core::settings::MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                    two_typos: m.two_typos.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                }
            }),
            disable_on_words: config.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: config.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: config.disable_on_numbers,
        };
        idx.update_typo_tolerance(lib_typo)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_typo_tolerance(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_typo_tolerance()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_pagination(&self, index_uid: &str) -> Result<Pagination> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_pagination()?
            .map(|p| Pagination {
                max_total_hits: p.max_total_hits.map(usize_to_u64),
            })
            .unwrap_or_default())
    }
    fn update_pagination(&self, index_uid: &str, config: &Pagination) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_pagination(crate::core::settings::PaginationSettings {
            max_total_hits: config.max_total_hits.map(|v| v as usize),
        })?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_pagination(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_pagination()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_faceting(&self, index_uid: &str) -> Result<Faceting> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_faceting()?
            .map(|f| Faceting {
                max_values_per_facet: f.max_values_per_facet.map(usize_to_u64),
                sort_facet_values_by: f.sort_facet_values_by.map(|m| {
                    m.into_iter()
                        .map(|(k, v)| {
                            let s = match v {
                                crate::core::settings::FacetValuesSort::Count => FacetValuesSort::Count,
                                crate::core::settings::FacetValuesSort::Alpha => FacetValuesSort::Alpha,
                            };
                            (k, s)
                        })
                        .collect()
                }),
            })
            .unwrap_or_default())
    }
    fn update_faceting(&self, index_uid: &str, config: &Faceting) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let sort_map = if let Some(m) = config.sort_facet_values_by.as_ref() {
            let mut result = std::collections::BTreeMap::new();
            for (k, v) in m {
                let sort = match v {
                    FacetValuesSort::Alpha => crate::core::settings::FacetValuesSort::Alpha,
                    FacetValuesSort::Count => crate::core::settings::FacetValuesSort::Count,
                };
                result.insert(k.clone(), sort);
            }
            Some(result)
        } else {
            None
        };
        idx.update_faceting(crate::core::settings::FacetingSettings {
            max_values_per_facet: config.max_values_per_facet.map(|v| v as usize),
            sort_facet_values_by: sort_map,
        })?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_faceting(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_faceting()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_dictionary(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_dictionary()?.map(|d| d.into_iter().collect()).unwrap_or_default())
    }
    fn update_dictionary(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_dictionary(words.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_dictionary(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_dictionary()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_separator_tokens()?.map(|t| t.into_iter().collect()).unwrap_or_default())
    }
    fn update_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_separator_tokens(tokens.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_separator_tokens()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_non_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_non_separator_tokens()?.map(|t| t.into_iter().collect()).unwrap_or_default())
    }
    fn update_non_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_non_separator_tokens(tokens.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_non_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_non_separator_tokens()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_proximity_precision(&self, index_uid: &str) -> Result<ProximityPrecision> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_proximity_precision()?
            .map(|p| match p {
                crate::core::settings::ProximityPrecision::ByWord => ProximityPrecision::ByWord,
                crate::core::settings::ProximityPrecision::ByAttribute => ProximityPrecision::ByAttribute,
            })
            .unwrap_or(ProximityPrecision::ByWord))
    }
    fn update_proximity_precision(&self, index_uid: &str, precision: ProximityPrecision) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let p = match precision {
            ProximityPrecision::ByWord => crate::core::settings::ProximityPrecision::ByWord,
            ProximityPrecision::ByAttribute => crate::core::settings::ProximityPrecision::ByAttribute,
        };
        idx.update_proximity_precision(p)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_proximity_precision(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_proximity_precision()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_facet_search(&self, index_uid: &str) -> Result<bool> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_facet_search()?.unwrap_or(true))
    }
    fn update_facet_search(&self, index_uid: &str, enabled: bool) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_facet_search(enabled)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_facet_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_facet_search()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_prefix_search(&self, index_uid: &str) -> Result<PrefixSearch> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_prefix_search()?
            .map(|s| match s.as_str() {
                "disabled" => PrefixSearch::Disabled,
                _ => PrefixSearch::IndexingTime,
            })
            .unwrap_or(PrefixSearch::IndexingTime))
    }
    fn update_prefix_search(&self, index_uid: &str, mode: PrefixSearch) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let mode_str = match mode {
            PrefixSearch::IndexingTime => "indexingTime".to_string(),
            PrefixSearch::Disabled => "disabled".to_string(),
        };
        idx.update_prefix_search(mode_str)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_prefix_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_prefix_search()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_search_cutoff_ms(&self, index_uid: &str) -> Result<Option<u64>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_search_cutoff_ms()?)
    }
    fn update_search_cutoff_ms(&self, index_uid: &str, ms: u64) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_search_cutoff_ms(ms)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_search_cutoff_ms(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_search_cutoff_ms()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_localized_attributes(
        &self,
        index_uid: &str,
    ) -> Result<Option<Vec<LocalizedAttribute>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_localized_attributes()?.map(|rules| {
            rules
                .into_iter()
                .map(|r| LocalizedAttribute {
                    locales: r.locales,
                    attribute_patterns: r.attribute_patterns,
                })
                .collect()
        }))
    }
    fn update_localized_attributes(
        &self,
        index_uid: &str,
        attrs: &[LocalizedAttribute],
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_rules: Vec<crate::core::settings::LocalizedAttributeRule> = attrs
            .iter()
            .map(|a| crate::core::settings::LocalizedAttributeRule {
                attribute_patterns: a.attribute_patterns.clone(),
                locales: a.locales.clone(),
            })
            .collect();
        idx.update_localized_attributes(lib_rules)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_localized_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_localized_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_embedders(
        &self,
        index_uid: &str,
    ) -> Result<Option<HashMap<String, EmbedderConfig>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_embedders()?.map(|embs| {
            embs.into_iter()
                .map(|(k, v)| {
                    let source = v
                        .source
                        .as_ref()
                        .map(|s| embedder_source_to_str(s).to_string())
                        .unwrap_or_default();
                    (
                        k,
                        EmbedderConfig {
                            source,
                            api_key: v.api_key,
                            model: v.model,
                            dimensions: v.dimensions.map(saturating_u32),
                            url: v.url,
                            extra: HashMap::new(),
                        },
                    )
                })
                .collect()
        }))
    }
    fn update_embedders(
        &self,
        index_uid: &str,
        embedders: &HashMap<String, EmbedderConfig>,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_embs: HashMap<String, crate::core::EmbedderSettings> = embedders
            .iter()
            .map(|(k, v)| {
                let mut es = crate::core::EmbedderSettings::default();
                let source = parse_embedder_source(&v.source);
                es.source = Some(source);
                es.api_key = v.api_key.clone();
                es.model = v.model.clone();
                es.dimensions = v.dimensions.map(|d| d as usize);
                es.url = v.url.clone();
                (k.clone(), es)
            })
            .collect();
        idx.update_embedders(lib_embs)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_embedders(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_embedders()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
}

// ─── Keys (stub -- no auth in embedded mode) ─────────────────────────────────

impl traits::Keys for Engine {
    fn list_keys(&self, _query: &PaginationQuery) -> Result<ApiKeyList> {
        Ok(ApiKeyList {
            results: vec![],
            offset: 0,
            limit: 20,
            total: 0,
        })
    }
    fn get_key(&self, _key_id: &str) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn create_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn update_key(&self, _key_id: &str, _request: &UpdateApiKeyRequest) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn delete_key(&self, _key_id: &str) -> Result<()> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
}

// ─── Webhooks (stub -- no webhooks in embedded mode) ─────────────────────────

impl traits::Webhooks for Engine {
    fn list_webhooks(&self) -> Result<Vec<Webhook>> {
        Ok(vec![])
    }
    fn get_webhook(&self, _webhook_uid: &str) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn create_webhook(&self, _request: &CreateWebhookRequest) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn update_webhook(&self, _uid: &str, _request: &UpdateWebhookRequest) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn delete_webhook(&self, _webhook_uid: &str) -> Result<()> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
}

// ─── System ──────────────────────────────────────────────────────────────────

impl traits::System for Engine {
    fn global_stats(&self) -> Result<GlobalStats> {
        let lib_stats = self.inner.stats()?;
        Ok(GlobalStats {
            database_size: lib_stats.database_size,
            used_database_size: None,
            last_update: lib_stats.last_update,
            indexes: lib_stats
                .indexes
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        IndexStats {
                            number_of_documents: v.number_of_documents,
                            is_indexing: v.is_indexing,
                            field_distribution: v.field_distribution.into_iter().collect(),
                        },
                    )
                })
                .collect(),
        })
    }

    fn index_stats(&self, index_uid: &str) -> Result<IndexStats> {
        let lib_stats = self.inner.index_stats(index_uid)?;
        Ok(IndexStats {
            number_of_documents: lib_stats.number_of_documents,
            is_indexing: lib_stats.is_indexing,
            field_distribution: lib_stats.field_distribution.into_iter().collect(),
        })
    }

    fn version(&self) -> Result<Version> {
        let v = self.inner.version();
        Ok(Version {
            commit_sha: v.commit_sha,
            commit_date: v.commit_date,
            pkg_version: v.pkg_version,
        })
    }

    fn health(&self) -> Result<Health> {
        let h = self.inner.health();
        Ok(Health { status: h.status })
    }

    fn create_dump(&self) -> Result<TaskInfo> {
        let _guard = self.dump_lock.lock().unwrap_or_else(|e| {
            tracing::warn!("dump_lock Mutex poisoned in create_dump, recovering");
            e.into_inner()
        });
        std::fs::create_dir_all(&self.dump_dir)?;
        self.inner.create_dump(&self.dump_dir)?;
        Ok(self.next_task("dumpCreation", None))
    }

    fn create_snapshot(&self) -> Result<TaskInfo> {
        let _guard = self.dump_lock.lock().unwrap_or_else(|e| {
            tracing::warn!("dump_lock Mutex poisoned in create_snapshot, recovering");
            e.into_inner()
        });
        std::fs::create_dir_all(&self.snapshot_dir)?;
        self.inner.create_snapshot(&self.snapshot_dir)?;
        Ok(self.next_task("snapshotCreation", None))
    }

    fn export(&self, request: &ExportRequest) -> Result<TaskInfo> {
        let raw_path = std::path::Path::new(&request.url);

        // Canonicalize the export path to prevent directory traversal.
        // Create the directory first so canonicalize has a real path to resolve.
        std::fs::create_dir_all(raw_path)?;
        let export_path = raw_path.canonicalize().map_err(|e| {
            crate::core::error::Error::Internal(format!(
                "Failed to canonicalize export path '{}': {e}",
                request.url
            ))
        })?;

        // Belt-and-suspenders: canonicalize() resolves symlinks and removes
        // `..` components, so this check should never trigger. Kept as
        // defense-in-depth against hypothetical platform edge cases.
        if export_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(crate::core::error::Error::Internal(
                "Export path must not contain '..' components".to_string(),
            ).into());
        }

        let index_settings: Option<HashMap<String, bool>> = request.indexes.as_ref().map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.override_settings.unwrap_or(true)))
                .collect()
        });

        self.inner.export(&export_path, index_settings.as_ref())?;
        Ok(self.next_task("export", None))
    }
}

// ─── Experimental features ───────────────────────────────────────────────────

impl traits::ExperimentalFeaturesApi for Engine {
    fn get_experimental_features(&self) -> Result<ExperimentalFeatures> {
        let lib_features = self.inner.get_experimental_features();
        Ok(ExperimentalFeatures {
            features: serde_json::from_value(serde_json::to_value(&lib_features)?)?,
        })
    }

    fn update_experimental_features(
        &self,
        features: &ExperimentalFeatures,
    ) -> Result<ExperimentalFeatures> {
        let lib_features: crate::core::meilisearch::ExperimentalFeatures =
            serde_json::from_value(serde_json::to_value(&features.features)?)?;
        let updated = self.inner.update_experimental_features(lib_features);
        Ok(ExperimentalFeatures {
            features: serde_json::from_value(serde_json::to_value(&updated)?)?,
        })
    }
}

// Static assertions: Engine must be Send + Sync for safe sharing across threads.
const _: () = {
    #[allow(dead_code)]
    fn assert_send_sync<T: Send + Sync>() {}
    #[allow(dead_code)]
    fn assert_engine() { assert_send_sync::<Engine>(); }
};
