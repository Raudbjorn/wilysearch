//! Type conversion functions between the public API types (`crate::types::*`)
//! and internal core types (`crate::core::*`).

use std::collections::HashMap;

use crate::traits::Result;
use crate::types::*;

use super::{saturating_u32, saturating_u128_to_u64, usize_to_u64};

pub(super) fn convert_search_request(req: &SearchRequest) -> crate::core::SearchQuery {
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
    q.hybrid = req.hybrid.as_ref().map(|h| crate::core::HybridQuery {
        semantic_ratio: h.semantic_ratio,
        embedder: h.embedder.clone(),
    });
    q.media = req.media.clone();
    q
}

pub(super) fn convert_search_result(r: &crate::core::SearchResult) -> Result<SearchResponse> {
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
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect(),
                )
            })
            .collect()
    });
    let facet_stats = r
        .facet_stats
        .as_ref()
        .map(|fs| serde_json::to_value(fs))
        .transpose()?;

    Ok(SearchResponse {
        degraded: r.degraded,
        used_negative_operator: r.used_negative_operator,
        semantic_hit_count: r.semantic_hit_count,
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

pub(super) fn convert_hit(h: &crate::core::SearchHit) -> Result<serde_json::Value> {
    // The SearchHit has `document` flattened, plus optional metadata fields.
    // We serialize the whole hit to produce the correct shape.
    Ok(serde_json::to_value(h)?)
}

pub(super) fn convert_federation_settings(
    s: &FederationSettings,
) -> crate::core::search::Federation {
    crate::core::search::Federation {
        distinct: s.distinct.clone(),
        limit: s.limit.unwrap_or(20) as usize,
        offset: s.offset.unwrap_or(0) as usize,
        page: s.page.map(|p| p as usize),
        hits_per_page: s.hits_per_page.map(|h| h as usize),
        facets_by_index: s
            .facets_by_index
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        merge_facets: s.merge_facets.map(|mf| crate::core::search::MergeFacets {
            max_values_per_facet: mf.max_values_per_facet.map(|v| v as usize),
        }),
    }
}

pub(super) fn convert_federated_result(
    r: &crate::core::search::FederatedSearchResult,
) -> Result<FederatedSearchResponse> {
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
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect(),
                )
            })
            .collect()
    });
    let facet_stats = r
        .facet_stats
        .as_ref()
        .map(|fs| serde_json::to_value(fs))
        .transpose()?;

    let facets_by_index: HashMap<String, serde_json::Value> = r
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
