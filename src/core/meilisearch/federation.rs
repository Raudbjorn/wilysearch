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
            let result = self.search(&msq.index_uid, &msq.query)?;
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
    /// Uses native weighted ranking tuples, document deduplication, and pin placement.
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

        use milli::score_details::{ScoreDetails, WeightedScoreValue};
        use std::collections::{BTreeMap, HashMap, HashSet};
        let mut entries = Vec::new();
        let mut previous_groups = Vec::new();
        let mut universes: HashMap<String, roaring::RoaringBitmap> = HashMap::new();
        for (position, fmq) in queries.iter().enumerate() {
            let weight = fmq.federation_options.as_ref().map_or(1.0, |o| o.weight);
            if !weight.is_finite() || weight < 0.0 {
                return Err(crate::core::Error::Internal(
                    "Federation weights must be finite and nonnegative".into(),
                ));
            }
            let index = self.get_index(&fmq.index_uid)?;
            let txn = index.inner.read_txn()?;
            let max_hits = index.inner.pagination_max_total_hits(&txn)?.unwrap_or(1000) as usize;
            let fields = index.inner.fields_ids_map(&txn)?;
            let groups = ranking_groups(&index.inner.criteria(&txn)?, &fmq.query)?;
            if groups.iter().zip(&previous_groups).any(|(a, b)| a != b) {
                return Err(crate::core::Error::Internal(
                    "Federated queries use incompatible ranking rules or sort directions".into(),
                ));
            }
            if groups.len() > previous_groups.len() {
                previous_groups = groups;
            }
            let mut query = fmq.query.clone();
            query.offset = 0;
            query.limit = max_hits;
            query.page = None;
            query.hits_per_page = None;
            query.show_ranking_score = true;
            query.show_ranking_score_details = true;
            if let Some(distinct) = &federation.distinct {
                query.distinct = Some(distinct.clone());
            }
            query.facets = None;
            // ponytail: merge up to maxTotalHits per query in memory; use an incremental merge if this bound becomes expensive.
            let result = self.search(&fmq.index_uid, &query)?;
            *universes.entry(fmq.index_uid.clone()).or_default() |= result.candidates;
            for ((mut hit, id), scores) in result
                .hits
                .into_iter()
                .zip(result.document_ids)
                .zip(result.scores)
            {
                let distinct = federation
                    .distinct
                    .as_ref()
                    .map(|field| {
                        let doc = index.make_document(
                            &txn,
                            &fields,
                            id,
                            Some(std::slice::from_ref(field)),
                            false,
                            false,
                        )?;
                        Ok::<_, crate::core::Error>(
                            if doc.as_object().is_some_and(|o| o.is_empty()) {
                                None
                            } else {
                                Some(serde_json::to_string(&doc)?)
                            },
                        )
                    })
                    .transpose()?
                    .flatten();
                let global = ScoreDetails::global_score(scores.iter()) * weight;
                let position = fmq
                    .federation_options
                    .as_ref()
                    .and_then(|o| o.query_position)
                    .unwrap_or(position);
                hit.document.as_object_mut().expect("document object").insert("_federation".into(), serde_json::json!({
                    "indexUid": fmq.index_uid, "queriesPosition": [position], "weightedRankingScore": global
                }));
                if !fmq.query.show_ranking_score {
                    hit.ranking_score = None;
                }
                if !fmq.query.show_ranking_score_details {
                    hit.ranking_score_details = None;
                }
                entries.push((
                    fmq.index_uid.clone(),
                    id,
                    scores,
                    weight,
                    global,
                    position,
                    hit,
                    distinct,
                ));
            }
        }
        let pin = |scores: &[ScoreDetails]| {
            scores.iter().find_map(|score| match score {
                ScoreDetails::Pin {
                    position,
                    precedence,
                    ..
                } => Some((milli::search::Precedence(*precedence), *position)),
                _ => None,
            })
        };
        entries.sort_by(|a, b| {
            pin(&a.2)
                .is_none()
                .cmp(&pin(&b.2).is_none())
                .then_with(|| pin(&a.2).cmp(&pin(&b.2)))
                .then_with(|| {
                    WeightedScoreValue::compare_partial(
                        ScoreDetails::weighted_score_values(b.2.iter(), b.3),
                        ScoreDetails::weighted_score_values(a.2.iter(), a.3),
                    )
                    .unwrap_or_else(|| b.4.total_cmp(&a.4))
                })
                .then(a.5.cmp(&b.5))
                .then(a.1.cmp(&b.1))
        });
        // A document found by several queries is returned once, with all contributing positions.
        let mut deduped = Vec::new();
        let mut identities = HashMap::new();
        let mut distinct_values = HashSet::new();
        for entry in entries {
            let identity = (entry.0.clone(), entry.1);
            if let Some(&i) = identities.get(&identity) {
                let old: &mut (_, _, _, _, _, _, crate::core::SearchHit, _) = &mut deduped[i];
                if let Some(positions) = old
                    .6
                    .document
                    .pointer_mut("/_federation/queriesPosition")
                    .and_then(|v| v.as_array_mut())
                {
                    let value = serde_json::json!(entry.5);
                    if !positions.contains(&value) {
                        positions.push(value);
                    }
                }
                continue;
            }
            if entry
                .7
                .as_ref()
                .is_some_and(|v| !distinct_values.insert(v.clone()))
            {
                continue;
            }
            identities.insert(identity, deduped.len());
            deduped.push(entry);
        }
        let total_hits = deduped.len();
        let mut pins = Vec::new();
        let mut organic = Vec::new();
        for entry in deduped {
            let pin = entry.2.iter().find_map(|score| match score {
                ScoreDetails::Pin {
                    position,
                    precedence,
                    ..
                } => Some((*position, milli::search::Precedence(*precedence))),
                _ => None,
            });
            let semantic = entry.2.iter().any(|s| matches!(s, ScoreDetails::Vector(_)));
            if let Some((position, precedence)) = pin {
                pins.push((position, precedence, (entry.6, semantic, true)));
            } else {
                organic.push((entry.6, semantic, false));
            }
        }
        pins.sort_by_key(|p| (p.0, p.1));
        let (skip, take) = if federation.page.is_some() || federation.hits_per_page.is_some() {
            let take = federation.hits_per_page.unwrap_or(20);
            (
                federation
                    .page
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .saturating_mul(take),
                take,
            )
        } else {
            (federation.offset, federation.limit)
        };
        let page_take = if federation.page == Some(0) { 0 } else { take };
        let hits = if pins.is_empty() {
            organic.into_iter().skip(skip).take(page_take).collect()
        } else {
            milli::search::merge_positioned_hits_into_page(
                pins.len(),
                pins,
                skip,
                page_take,
                organic,
                |p| p.0,
                |p| p.2,
            )
        };
        let semantic_count = hits.iter().filter(|(_, semantic, _)| *semantic).count() as u32;
        let pin_positions = hits
            .iter()
            .enumerate()
            .filter_map(|(i, (_, _, pin))| pin.then_some(i))
            .collect();
        let hits = hits.into_iter().map(|(hit, _, _)| hit).collect();
        let hits_info = if federation.page.is_some() || federation.hits_per_page.is_some() {
            HitsInfo::Pagination {
                page: federation.page.unwrap_or(1),
                hits_per_page: take,
                total_hits,
                total_pages: if take == 0 {
                    0
                } else {
                    total_hits.div_ceil(take)
                },
            }
        } else {
            HitsInfo::OffsetLimit {
                offset: skip,
                limit: take,
                estimated_total_hits: total_hits,
            }
        };
        let mut facets_by_index = BTreeMap::new();
        let mut facet_orders = BTreeMap::new();
        for (uid, facets) in &federation.facets_by_index {
            let Some(facets) = facets else { continue };
            if !universes.contains_key(uid) {
                return Err(crate::core::Error::Internal(format!(
                    "Facet index {uid} was not searched"
                )));
            }
            let index = self.get_index(uid)?;
            let txn = index.inner.read_txn()?;
            let fields = index.inner.fields_ids_map(&txn)?;
            let orders = index.inner.sort_facet_values_by(&txn)?;
            let mut distribution = milli::FacetDistribution::new(&txn, &index.inner, &fields);
            distribution.facets(facets.iter().map(|f| (f, orders.get(f))));
            distribution.max_values_per_facet(if federation.merge_facets.is_some() {
                usize::MAX
            } else {
                index.inner.max_values_per_facet(&txn)?.unwrap_or(100) as usize
            });
            distribution.candidates(universes.remove(uid).unwrap_or_default());
            let values = distribution.execute()?;
            for field in values.keys() {
                let order = orders.get(field);
                if let Some(previous) = facet_orders.insert(field.clone(), order) {
                    if previous != order && federation.merge_facets.is_some() {
                        return Err(crate::core::Error::Internal(format!(
                            "Conflicting facet order for {field}"
                        )));
                    }
                }
            }
            facets_by_index.insert(
                uid.clone(),
                ComputedFacets {
                    distribution: values,
                    stats: distribution
                        .compute_stats()?
                        .into_iter()
                        .map(|(k, (min, max))| (k, crate::core::FacetStats { min, max }))
                        .collect(),
                },
            );
        }
        let (facet_distribution, facet_stats) = if let Some(merge) = federation.merge_facets {
            let mut merged: BTreeMap<String, indexmap::IndexMap<String, u64>> = BTreeMap::new();
            let mut stats: BTreeMap<String, crate::core::FacetStats> = BTreeMap::new();
            for facets in facets_by_index.values() {
                for (field, values) in &facets.distribution {
                    for (value, count) in values {
                        *merged
                            .entry(field.clone())
                            .or_default()
                            .entry(value.clone())
                            .or_default() += count;
                    }
                }
                for (field, range) in &facets.stats {
                    stats
                        .entry(field.clone())
                        .and_modify(|s| {
                            s.min = s.min.min(range.min);
                            s.max = s.max.max(range.max);
                        })
                        .or_insert(range.clone());
                }
            }
            for (field, values) in &mut merged {
                if facet_orders.get(field) == Some(&milli::OrderBy::Count) {
                    values.sort_by(|a, ac, b, bc| bc.cmp(ac).then(a.cmp(b)));
                } else {
                    values.sort_keys();
                }
                values.truncate(merge.max_values_per_facet.unwrap_or(100));
            }
            facets_by_index.clear();
            (Some(merged), Some(stats))
        } else {
            (None, None)
        };
        Ok(FederatedSearchResult {
            pin_positions,
            hits,
            processing_time_ms: start_time.elapsed().as_millis(),
            hits_info,
            facet_distribution,
            facet_stats,
            facets_by_index,
            semantic_hit_count: queries
                .iter()
                .any(|q| q.query.hybrid.is_some())
                .then_some(semantic_count),
        })
    }
}

// Upstream federation compares groups of relevancy rules and individual sort rules.
// Field names may differ across indexes; sort directions and group positions must agree.
#[derive(PartialEq, Eq)]
enum RankingGroup {
    Relevance,
    Asc,
    Desc,
    GeoAsc,
    GeoDesc,
}
fn ranking_groups(
    criteria: &[milli::Criterion],
    query: &crate::core::SearchQuery,
) -> Result<Vec<RankingGroup>> {
    use RankingGroup::*;
    use milli::{AscDesc, Criterion, Member};
    let vector = query
        .hybrid
        .as_ref()
        .is_some_and(|h| h.semantic_ratio == 1.0);
    let keyword = query.q.as_ref().is_some_and(|q| !q.trim().is_empty());
    let mut result = Vec::new();
    let mut seen = Vec::new();
    let mut fields = std::collections::HashSet::new();
    let mut geo = false;
    let mut vector_seen = false;
    for criterion in criteria {
        if seen.contains(criterion) {
            continue;
        }
        seen.push(criterion.clone());
        match criterion {
            Criterion::Sort => {
                for sort in query.sort.iter().flatten() {
                    let sort: AscDesc = sort
                        .parse()
                        .map_err(|e| crate::core::Error::InvalidSort(format!("{e}")))?;
                    let asc = matches!(sort, AscDesc::Asc(_));
                    match sort {
                        AscDesc::Asc(Member::Field(field))
                        | AscDesc::Desc(Member::Field(field)) => {
                            if fields.insert(field) {
                                result.push(if asc { Asc } else { Desc });
                            }
                        }
                        _ if !geo => {
                            geo = true;
                            result.push(if asc { GeoAsc } else { GeoDesc });
                        }
                        _ => {}
                    }
                }
            }
            Criterion::Asc(field) | Criterion::Desc(field) => {
                if fields.insert(field.clone()) {
                    result.push(match (field.as_str(), criterion) {
                        ("_geo", Criterion::Asc(_)) => GeoAsc,
                        ("_geo", _) => GeoDesc,
                        (_, Criterion::Asc(_)) => Asc,
                        _ => Desc,
                    });
                }
            }
            _ if vector => {
                if vector_seen {
                    if !fields.is_empty() {
                        break;
                    }
                } else {
                    result.push(Relevance);
                    vector_seen = true;
                }
            }
            _ if keyword => {
                if *criterion == Criterion::Words
                    && query.matching_strategy == crate::core::MatchingStrategy::All
                {
                    continue;
                }
                // The first non-words relevance rule implicitly inserts words, unless matching all terms.
                if query.matching_strategy != crate::core::MatchingStrategy::All
                    && !seen.contains(&Criterion::Words)
                {
                    seen.push(Criterion::Words);
                }
                if result.last() != Some(&Relevance) {
                    result.push(Relevance);
                }
            }
            _ => {}
        }
    }
    Ok(result)
}
