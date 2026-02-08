//! `Search` trait implementation for `Engine`.

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;
use super::conversion::*;
use super::{saturating_u32, saturating_u128_to_u64, usize_to_u64};

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
            id: serde_json::Value::String(result.id),
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
