//! `Search` trait implementation for `Engine`.

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;
use super::conversion::*;
use super::{saturating_u32, saturating_u128_to_u64, usize_to_u64};

impl traits::Search for Engine {
    fn search(&self, index_uid: &str, request: &SearchRequest) -> Result<SearchResponse> {
        let start = std::time::Instant::now();
        validate_request(request)?;
        let query = convert_search_request(request);
        let result = self.inner.search(index_uid, &query)?;
        let mut response = convert_search_result(&result)?;
        if let Some(personalize) = &request.personalize {
            let pins = result
                .scores
                .iter()
                .enumerate()
                .filter_map(|(i, scores)| {
                    scores
                        .iter()
                        .any(|s| matches!(s, milli::score_details::ScoreDetails::Pin { .. }))
                        .then_some(i)
                })
                .collect::<Vec<_>>();
            self.personalize(
                &mut response.hits,
                request.q.as_deref(),
                personalize,
                &[index_uid],
                result.processing_time_ms,
                &pins,
            )?;
        }
        response.processing_time_ms = saturating_u128_to_u64(start.elapsed().as_millis());
        Ok(response)
    }

    fn similar(&self, index_uid: &str, request: &SimilarRequest) -> Result<SimilarResponse> {
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
        let result = self.inner.similar(index_uid, &lib_query)?;

        let (offset, limit, estimated) = match &result.hits_info {
            crate::core::search::HitsInfo::OffsetLimit {
                offset,
                limit,
                estimated_total_hits,
            } => (
                saturating_u32(*offset),
                saturating_u32(*limit),
                usize_to_u64(*estimated_total_hits),
            ),
            crate::core::search::HitsInfo::Pagination { total_hits, .. } => {
                let req_offset = request.offset.unwrap_or(0) as usize;
                let req_limit = request.limit.unwrap_or(20) as usize;
                (
                    saturating_u32(req_offset),
                    saturating_u32(req_limit),
                    usize_to_u64(*total_hits),
                )
            }
        };

        Ok(SimilarResponse {
            hits: result
                .hits
                .iter()
                .map(convert_hit)
                .collect::<Result<Vec<_>>>()?,
            offset,
            limit,
            estimated_total_hits: estimated,
            processing_time_ms: saturating_u128_to_u64(result.processing_time_ms),
            id: serde_json::Value::String(result.id),
        })
    }

    fn multi_search(&self, request: &MultiSearchRequest) -> Result<MultiSearchResult> {
        let start = std::time::Instant::now();
        if let Some(ref federation) = request.federation {
            if (federation.page.is_some() || federation.hits_per_page.is_some())
                && (federation.offset.is_some() || federation.limit.is_some())
            {
                return Err(crate::core::Error::InvalidPagination(
                    "Choose page/hitsPerPage or offset/limit".into(),
                ));
            }
            for query in &request.queries {
                if query
                    .federation_options
                    .as_ref()
                    .is_some_and(|o| o.remote.is_some())
                {
                    return Err(crate::core::Error::Internal(
                        "Remote federation is not supported in embedded mode".into(),
                    ));
                }
                validate_request(&query.search)?;
                if query.search.limit.is_some()
                    || query.search.offset.is_some()
                    || query.search.page.is_some()
                    || query.search.hits_per_page.is_some()
                    || query.search.facets.is_some()
                    || query.search.personalize.is_some()
                {
                    return Err(crate::core::Error::Internal(
                        "Use federation-level pagination, facets, and personalization".into(),
                    ));
                }
            }
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

            let result = self
                .inner
                .multi_search_federated(core_queries, core_federation)?;
            let mut response = convert_federated_result(&result)?;
            if let Some(personalize) = &federation.personalize {
                let indexes = request
                    .queries
                    .iter()
                    .map(|q| q.index_uid.as_str())
                    .collect::<Vec<_>>();
                let query = request
                    .queries
                    .iter()
                    .filter_map(|q| q.search.q.as_deref())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.personalize(
                    &mut response.hits,
                    Some(&query),
                    personalize,
                    &indexes,
                    result.processing_time_ms,
                    &result.pin_positions,
                )?;
            }
            response.processing_time_ms = saturating_u128_to_u64(start.elapsed().as_millis());
            Ok(MultiSearchResult::Federated(response))
        } else {
            let mut results = Vec::with_capacity(request.queries.len());
            for mq in &request.queries {
                if mq.federation_options.is_some() {
                    return Err(crate::core::Error::Internal(
                        "federationOptions require federation".into(),
                    ));
                }
                let resp = self.search(&mq.index_uid, &mq.search)?;
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
            ranking_score_threshold: request.ranking_score_threshold,
            locales: request.locales.clone(),
        };
        let result = self.inner.facet_search(index_uid, &lib_query)?;
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

impl Engine {
    fn personalize(
        &self,
        hits: &mut Vec<serde_json::Value>,
        query: Option<&str>,
        personalize: &Personalize,
        indexes: &[&str],
        elapsed_ms: u128,
        pins: &[usize],
    ) -> Result<()> {
        #[cfg(not(feature = "ai"))]
        {
            let _ = (hits, query, personalize, indexes, elapsed_ms, pins);
            Err(crate::core::Error::ExperimentalFeatureNotEnabled(
                "ai Cargo feature".into(),
            ))
        }
        #[cfg(feature = "ai")]
        {
            let reranker = self
                .personalization
                .read()
                .map_err(|_| crate::core::Error::Internal("Personalization lock poisoned".into()))?
                .clone()
                .ok_or_else(|| {
                    crate::core::Error::Internal("Cohere personalization is not configured".into())
                })?;
            let mut budget = std::time::Duration::from_secs(30);
            for uid in indexes {
                let index = self.resolve_index(uid)?;
                let txn = index.inner.read_txn()?;
                if let Some(deadline) = index.inner.search_deadline(&txn)?.to_instant() {
                    budget =
                        budget.min(deadline.saturating_duration_since(std::time::Instant::now()));
                }
            }
            let remaining = budget.saturating_sub(std::time::Duration::from_millis(
                u64::try_from(elapsed_ms).unwrap_or(u64::MAX),
            ));
            let positions: Vec<usize> = (0..hits.len()).filter(|i| !pins.contains(i)).collect();
            let organic: Vec<_> = positions.iter().map(|&i| hits[i].clone()).collect();
            let documents = organic
                .iter()
                .cloned()
                .map(|mut hit| {
                    if let Some(object) = hit.as_object_mut() {
                        for key in [
                            "_rankingScore",
                            "_rankingScoreDetails",
                            "_formatted",
                            "_matchesPosition",
                            "_federation",
                        ] {
                            object.remove(key);
                        }
                    }
                    hit
                })
                .collect::<Vec<_>>();
            let order = reranker.order(
                query,
                &personalize.user_context,
                &documents,
                std::time::Instant::now() + remaining,
            )?;
            for (position, i) in positions.into_iter().zip(order) {
                hits[position] = organic[i].clone();
            }
            Ok(())
        }
    }
}

fn validate_request(request: &SearchRequest) -> Result<()> {
    if (request.page.is_some() || request.hits_per_page.is_some())
        && (request.limit.is_some() || request.offset.is_some())
    {
        return Err(crate::core::Error::InvalidPagination(
            "Choose page/hitsPerPage or offset/limit".into(),
        ));
    }
    if request.vector.as_ref().is_some_and(|v| {
        v.is_empty() || v.iter().any(|&f| !f.is_finite() || !(f as f32).is_finite())
    }) {
        return Err(crate::core::Error::InvalidSearchRequest(
            "Vector must contain finite f32 components".into(),
        ));
    }
    Ok(())
}
