use tracing::instrument;

use crate::core::error::Result;
use crate::core::search::{
    ComputedFacets, FederatedMultiSearchQuery, FederatedSearchResult, Federation, HitsInfo,
    MultiSearchQuery, MultiSearchResult, SearchResultWithIndex,
};

use super::Meilisearch;

impl Meilisearch {
    /// Execute multiple search queries across different indexes in a single call.
    ///
    /// Each query targets a specific index identified by `index_uid`. Results are
    /// returned in the same order as the input queries, each tagged with the
    /// index UID it came from.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// use wilysearch::core::search::{MultiSearchQuery, SearchQuery};
    ///
    /// let queries = vec![
    ///     MultiSearchQuery {
    ///         index_uid: "movies".to_string(),
    ///         query: SearchQuery::new("action"),
    ///     },
    ///     MultiSearchQuery {
    ///         index_uid: "books".to_string(),
    ///         query: SearchQuery::new("thriller"),
    ///     },
    /// ];
    ///
    /// let result = meili.multi_search(queries)?;
    /// for r in &result.results {
    ///     println!("{}: {} hits", r.index_uid, r.result.hits.len());
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    #[instrument(skip(self, queries))]
    pub fn multi_search(&self, queries: Vec<MultiSearchQuery>) -> Result<MultiSearchResult> {
        let mut results = Vec::with_capacity(queries.len());

        for msq in queries {
            let index = self.get_index(&msq.index_uid)?;
            let result = index.search(&msq.query)?;
            results.push(SearchResultWithIndex {
                index_uid: msq.index_uid,
                result,
            });
        }

        Ok(MultiSearchResult { results })
    }

    /// Execute a federated multi-search that merges hits from multiple indexes.
    ///
    /// Unlike [`multi_search`](Self::multi_search), federated search returns a
    /// single flat list of hits drawn from all queried indexes. Each query can
    /// carry optional [`FederationOptions`](crate::FederationOptions) (weight, query_position) that
    /// influence how its hits are ranked relative to hits from other queries.
    ///
    /// The `federation` parameter controls global result pagination
    /// (limit/offset or page/hits_per_page) and facet aggregation.
    ///
    /// **Note:** This is a basic implementation that concatenates hits from each
    /// query, sorts by ranking score when available, and applies the federation
    /// pagination. Full score normalization and cross-index ranking are not yet
    /// implemented.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// use wilysearch::core::search::{
    ///     FederatedMultiSearchQuery, Federation, FederationOptions, SearchQuery,
    /// };
    ///
    /// let queries = vec![
    ///     FederatedMultiSearchQuery {
    ///         index_uid: "movies".to_string(),
    ///         query: SearchQuery::new("action"),
    ///         federation_options: Some(FederationOptions { weight: 1.0, query_position: None }),
    ///     },
    ///     FederatedMultiSearchQuery {
    ///         index_uid: "books".to_string(),
    ///         query: SearchQuery::new("thriller"),
    ///         federation_options: Some(FederationOptions { weight: 0.8, query_position: None }),
    ///     },
    /// ];
    ///
    /// let federation = Federation { limit: 10, offset: 0, ..Default::default() };
    /// let result = meili.multi_search_federated(queries, federation)?;
    /// println!("Total merged hits: {}", result.hits.len());
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    #[instrument(skip(self, queries, federation))]
    pub fn multi_search_federated(
        &self,
        queries: Vec<FederatedMultiSearchQuery>,
        federation: Federation,
    ) -> Result<FederatedSearchResult> {
        let start_time = std::time::Instant::now();

        // Collect all hits from every query, weighted by federation_options.
        let mut all_hits = Vec::new();
        let mut facets_by_index: std::collections::BTreeMap<String, ComputedFacets> =
            std::collections::BTreeMap::new();
        let mut total_semantic_hits: u32 = 0;

        for fmq in &queries {
            let index = self.get_index(&fmq.index_uid)?;
            let result = index.search(&fmq.query)?;

            let weight = fmq
                .federation_options
                .as_ref()
                .map(|o| o.weight)
                .unwrap_or(1.0);

            // Scale ranking scores by weight and collect hits.
            for mut hit in result.hits {
                if let Some(score) = hit.ranking_score {
                    hit.ranking_score = Some(score * weight);
                }
                all_hits.push(hit);
            }

            // Aggregate per-index facets when present.
            if result.facet_distribution.is_some() || result.facet_stats.is_some() {
                facets_by_index.insert(
                    fmq.index_uid.clone(),
                    ComputedFacets {
                        distribution: result.facet_distribution.unwrap_or_default(),
                        stats: result.facet_stats.unwrap_or_default(),
                    },
                );
            }

            if let Some(shc) = result.semantic_hit_count {
                total_semantic_hits += shc;
            }
        }

        // Sort hits by ranking score (descending). Hits without a score sort last.
        all_hits.sort_by(|a, b| {
            let sa = a.ranking_score.unwrap_or(f64::NEG_INFINITY);
            let sb = b.ranking_score.unwrap_or(f64::NEG_INFINITY);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_hits = all_hits.len();

        // Apply federation-level pagination.
        let (hits_info, hits) = if let Some(page) = federation.page {
            let hpp = federation.hits_per_page.unwrap_or(20);
            let skip = (page.saturating_sub(1)) * hpp;
            let total_pages = if hpp > 0 {
                (total_hits + hpp - 1) / hpp
            } else {
                0
            };
            let paginated: Vec<_> = all_hits.into_iter().skip(skip).take(hpp).collect();
            (
                HitsInfo::Pagination {
                    hits_per_page: hpp,
                    page,
                    total_pages,
                    total_hits,
                },
                paginated,
            )
        } else {
            let paginated: Vec<_> = all_hits
                .into_iter()
                .skip(federation.offset)
                .take(federation.limit)
                .collect();
            (
                HitsInfo::OffsetLimit {
                    limit: federation.limit,
                    offset: federation.offset,
                    estimated_total_hits: total_hits,
                },
                paginated,
            )
        };

        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(FederatedSearchResult {
            hits,
            processing_time_ms,
            hits_info,
            facet_distribution: None,
            facet_stats: None,
            facets_by_index,
            semantic_hit_count: if total_semantic_hits > 0 {
                Some(total_semantic_hits)
            } else {
                None
            },
        })
    }
}
