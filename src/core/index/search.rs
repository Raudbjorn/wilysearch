use milli::score_details::{ScoreDetails, ScoringStrategy};
use milli::tokenizer::TokenizerBuilder;
use milli::{AscDesc, FacetDistribution, OrderBy, TermsMatchingStrategy};
use milli::{FormatOptions, MatcherBuilder};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Instant;
use tracing::instrument;

use crate::core::error::{Error, Result};
use crate::core::search::{FacetStats, MatchingStrategy, SearchHit, SearchQuery, SearchResult};

use super::Index;

impl Index {
    /// Perform a search using a SearchQuery and return a SearchResult.
    ///
    /// This is the primary search method that supports all search options including
    /// filtering, pagination, attribute selection, and ranking scores.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::SearchQuery;
    ///
    /// let query = SearchQuery::new("search terms")
    ///     .with_limit(10)
    ///     .with_filter("category = 'books'")
    ///     .with_ranking_score(true);
    ///
    /// let result = index.search(&query)?;
    /// for hit in result.hits {
    ///     println!("Score: {:?}, Doc: {}", hit.ranking_score, hit.document);
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    #[instrument(skip(self, query))]
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResult> {
        self.search_with_context(query, None, None)
    }

    pub(crate) fn search_with_context(
        &self,
        query: &SearchQuery,
        filter: Option<milli::IndexFilter>,
        rules: Option<&milli::dynamic_search_rules::DynamicSearchRules>,
    ) -> Result<SearchResult> {
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::quiet();
        let fields_ids_map = self.inner.fields_ids_map(&rtxn)?;

        let mut search = self.inner.search(
            &rtxn,
            &self.uid,
            &fields_ids_map,
            time::OffsetDateTime::now_utc(),
            &progress,
        );

        // Set query string if provided
        if query
            .hybrid
            .as_ref()
            .is_none_or(|h| h.semantic_ratio != 1.0)
        {
            if let Some(q) = &query.q {
                search.query(q);
            }
        }

        // ------------------------------------------------------------------
        // Pagination: page-based vs offset/limit
        // ------------------------------------------------------------------
        let use_page_pagination = query.page.is_some() || query.hits_per_page.is_some();
        let (effective_offset, effective_limit, page_val, hpp_val) = if use_page_pagination {
            let page = query.page.unwrap_or(1);
            let hits_per_page = query.hits_per_page.unwrap_or(20);
            let computed_offset = page.saturating_sub(1).saturating_mul(hits_per_page);
            (
                computed_offset,
                if page == 0 { 0 } else { hits_per_page },
                page,
                hits_per_page,
            )
        } else {
            (query.offset, query.limit, 0, 0)
        };

        let max_hits = self.inner.pagination_max_total_hits(&rtxn)?.unwrap_or(1000) as usize;
        search.offset(effective_offset);
        search.limit(effective_limit.min(max_hits.saturating_sub(effective_offset)));
        search.max_total_hits(Some(max_hits));
        search.deadline(self.inner.search_deadline(&rtxn)?);
        search.retrieve_vectors(query.retrieve_vectors);
        if query
            .ranking_score_threshold
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(Error::Internal(
                "rankingScoreThreshold must be between 0 and 1".into(),
            ));
        }

        // For page-based pagination we need exhaustive counts
        if use_page_pagination {
            search.exhaustive_number_hits(true);
        }

        // ------------------------------------------------------------------
        // Scoring strategy
        // ------------------------------------------------------------------
        if query.show_ranking_score
            || query.show_ranking_score_details
            || query.ranking_score_threshold.is_some()
        {
            search.scoring_strategy(ScoringStrategy::Detailed);
        }

        // ------------------------------------------------------------------
        // Terms matching strategy
        // ------------------------------------------------------------------
        let tms = match query.matching_strategy {
            MatchingStrategy::Last => TermsMatchingStrategy::Last,
            MatchingStrategy::All => TermsMatchingStrategy::All,
            MatchingStrategy::Frequency => TermsMatchingStrategy::Frequency,
        };
        search.terms_matching_strategy(tms);

        // ------------------------------------------------------------------
        // Filter (supports string, array-of-strings, and array-of-arrays)
        // ------------------------------------------------------------------
        search.filter(filter);
        if let Some(rules) = rules {
            search.dynamic_search_rules(rules, crate::core::meilisearch::rules::fuel());
        }
        if let Some(value) = &query.filter {
            if let Some(filter) = super::parse_local_filter(value)? {
                search.filter(Some(filter));
            }
        }

        // ------------------------------------------------------------------
        // Sort criteria
        // ------------------------------------------------------------------
        if let Some(sort_strings) = &query.sort {
            let mut criteria = Vec::with_capacity(sort_strings.len());
            for s in sort_strings {
                let asc_desc =
                    AscDesc::from_str(s).map_err(|e| Error::InvalidSort(format!("{e}")))?;
                criteria.push(asc_desc);
            }
            search.sort_criteria(criteria);
        }

        // ------------------------------------------------------------------
        // Distinct
        // ------------------------------------------------------------------
        if let Some(ref distinct) = query.distinct {
            search.distinct(distinct.clone());
        }

        // ------------------------------------------------------------------
        // Ranking score threshold
        // ------------------------------------------------------------------
        if let Some(threshold) = query.ranking_score_threshold {
            search.ranking_score_threshold(threshold);
        }

        // ------------------------------------------------------------------
        // Attributes to search on
        // ------------------------------------------------------------------
        let searchable_attrs_owned;
        if let Some(ref attrs) = query.attributes_to_search_on {
            searchable_attrs_owned = attrs.clone();
            search.searchable_attributes(&searchable_attrs_owned);
        }

        // ------------------------------------------------------------------
        // Locales
        // ------------------------------------------------------------------
        let parsed_locales = self.parse_locales(query.locales.as_deref())?;
        if let Some(ref locales) = parsed_locales {
            search.locales(locales.clone());
        }

        // ------------------------------------------------------------------
        // Execute search
        // ------------------------------------------------------------------
        let (result, semantic_count) = if let Some(hybrid) = &query.hybrid {
            let ratio = hybrid.semantic_ratio;
            if !ratio.is_finite() || !(0.0..=1.0).contains(&ratio) {
                return Err(Error::Internal(
                    "semanticRatio must be between 0 and 1".into(),
                ));
            }
            let settings = milli::update::InnerIndexSettings::from_index(
                &self.inner,
                &rtxn,
                &self.ip_policy.clone(),
                None,
            )?;
            let runtime = settings
                .runtime_embedders
                .get(&hybrid.embedder)
                .ok_or_else(|| Error::EmbedderNotFound(hybrid.embedder.clone()))?;
            if ratio == 1.0 {
                let vector = match &query.vector {
                    Some(vector) => vector.clone(),
                    None => runtime
                        .embedder
                        .embed_search(
                            match (query.q.as_deref(), query.media.as_ref()) {
                                (Some(text), None) => milli::vector::SearchQuery::Text(text),
                                (q, media) => milli::vector::SearchQuery::Media { q, media },
                            },
                            self.inner.search_deadline(&rtxn)?.to_instant(),
                        )
                        .map_err(milli::vector::Error::from)
                        .map_err(milli::Error::from)?,
                };
                search.semantic(
                    hybrid.embedder.clone(),
                    runtime.embedder.clone(),
                    runtime.is_quantized,
                    Some(vector),
                    query.media.clone(),
                );
                let result = search.execute()?;
                let count = result.documents_ids.len() as u32;
                (result, Some(count))
            } else {
                if ratio > 0.0 {
                    search.semantic(
                        hybrid.embedder.clone(),
                        runtime.embedder.clone(),
                        runtime.is_quantized,
                        query.vector.clone(),
                        query.media.clone(),
                    );
                }
                search.execute_hybrid(ratio)?
            }
        } else {
            if query.vector.is_some() || query.media.is_some() {
                return Err(Error::Internal(
                    "vector and media require a hybrid embedder".into(),
                ));
            }
            (search.execute()?, None)
        };
        let attribute_state = milli::AttributeState::from_criteria(self.inner.criteria(&rtxn)?);

        // Get the documents -- preserve milli IDs for hybrid search merging
        let returned_doc_ids = result.documents_ids.clone();
        let documents = self
            .inner
            .documents(&rtxn, result.documents_ids.clone())
            .map_err(Error::Milli)?;
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        // ------------------------------------------------------------------
        // Highlighting / formatting / matches_position setup
        // ------------------------------------------------------------------
        let needs_formatting =
            query.attributes_to_highlight.is_some() || query.attributes_to_crop.is_some();
        let needs_matches = query.show_matches_position;
        let needs_matcher = needs_formatting || needs_matches;

        // Build MatcherBuilder if needed (consumes matching_words from result)
        let tokenizer;
        let mut matcher_builder_storage;
        let stop_words = self
            .inner
            .stop_words(&rtxn)?
            .map(|s| fst::Set::new(s.as_fst().as_bytes().to_vec()).expect("stored FST is valid"));
        let separators = self.inner.allowed_separators(&rtxn)?;
        let dictionary = self.inner.dictionary(&rtxn)?;
        let separators = separators
            .as_ref()
            .map(|s| s.iter().map(String::as_str).collect::<Vec<_>>());
        let dictionary = dictionary
            .as_ref()
            .map(|s| s.iter().map(String::as_str).collect::<Vec<_>>());
        let matcher_builder_opt: Option<&MatcherBuilder<'_>> = if needs_matcher {
            let mut builder = TokenizerBuilder::default();
            if let Some(ref stop_words) = stop_words {
                builder.stop_words(stop_words);
            }
            if let Some(ref separators) = separators {
                builder.separators(separators);
            }
            if let Some(ref dictionary) = dictionary {
                builder.words_dict(dictionary);
            }
            tokenizer = builder.into_tokenizer();
            matcher_builder_storage = MatcherBuilder::new(result.matching_words, tokenizer);
            matcher_builder_storage.crop_marker(query.crop_marker.clone());
            matcher_builder_storage.highlight_prefix(query.highlight_pre_tag.clone());
            matcher_builder_storage.highlight_suffix(query.highlight_post_tag.clone());
            Some(&matcher_builder_storage)
        } else {
            None
        };

        // ------------------------------------------------------------------
        // Build hits
        // ------------------------------------------------------------------
        let mut hits = Vec::with_capacity(documents.len());
        for (idx, (id, _obkv)) in documents.into_iter().enumerate() {
            let requested = query
                .attributes_to_retrieve
                .as_ref()
                .map(|s| s.iter().cloned().collect::<Vec<_>>());
            let doc = self.make_document(
                &rtxn,
                &fields_ids_map,
                id,
                requested.as_deref(),
                query.retrieve_vectors,
                true,
            )?;
            let json = doc.as_object().cloned().unwrap_or_default();

            // Ranking score
            let ranking_score = if query.show_ranking_score {
                result
                    .document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::global_score(scores.iter()))
            } else {
                None
            };

            // Ranking score details
            let ranking_score_details = if query.show_ranking_score_details {
                result
                    .document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::to_json_map(attribute_state, scores.iter()))
            } else {
                None
            };

            // Formatting and matches_position
            let mut formatted = None;
            let mut matches_position = None;

            if let Some(mb) = matcher_builder_opt {
                let mut formatted_doc = json.clone();
                let mut all_matches: BTreeMap<String, Vec<crate::core::search::MatchBounds>> =
                    BTreeMap::new();

                let mut formatted_value = Value::Object(formatted_doc);
                format_document(
                    &mut formatted_value,
                    "",
                    &mut Vec::new(),
                    mb,
                    query,
                    parsed_locales.as_deref(),
                    &mut all_matches,
                );
                formatted_doc = formatted_value.as_object().cloned().unwrap_or_default();

                if needs_formatting {
                    formatted = Some(Value::Object(formatted_doc));
                }
                if needs_matches && !all_matches.is_empty() {
                    matches_position = Some(all_matches);
                }
            }

            let mut hit = SearchHit::new(doc, ranking_score);
            hit.ranking_score_details = ranking_score_details;
            hit.formatted = formatted;
            hit.matches_position = matches_position;

            hits.push(hit);
        }

        let processing_time_ms = start_time.elapsed().as_millis();
        // RoaringBitmap::len() returns u64; on 64-bit targets this is lossless.
        // On 32-bit targets this would truncate, but wilysearch targets x86_64.
        let total_hits = (result.candidates.len() as usize).min(max_hits);
        let query_string = query.q.clone().unwrap_or_default();

        // ------------------------------------------------------------------
        // Build result (page-based or offset/limit)
        // ------------------------------------------------------------------
        let mut search_result = if use_page_pagination {
            let mut r = SearchResult::new_paginated(
                hits,
                query_string,
                processing_time_ms,
                total_hits,
                page_val,
                hpp_val,
            );
            r.document_ids = returned_doc_ids;
            r
        } else {
            SearchResult::with_document_ids(
                hits,
                returned_doc_ids,
                query_string,
                processing_time_ms,
                total_hits,
                effective_limit,
                effective_offset,
            )
        };

        // ------------------------------------------------------------------
        // Facet distribution and stats
        // ------------------------------------------------------------------
        search_result.candidates = result.candidates.clone();
        search_result.semantic_hit_count = semantic_count;
        search_result.degraded = result.degraded;
        search_result.used_negative_operator = result.used_negative_operator;
        search_result.scores = result.document_scores;
        if let Some(ref facet_names) = query.facets {
            let mut fd = FacetDistribution::new(&rtxn, &self.inner, &fields_ids_map);
            let orders = self.inner.sort_facet_values_by(&rtxn)?;
            fd.max_values_per_facet(self.inner.max_values_per_facet(&rtxn)?.unwrap_or(100) as usize);
            let facets_with_order: Vec<(String, OrderBy)> = facet_names
                .iter()
                .map(|name| (name.clone(), orders.get(name)))
                .collect();
            fd.facets(facets_with_order);
            fd.candidates(result.candidates.clone());

            let distribution = fd.execute().map_err(Error::Milli)?;
            let stats = fd.compute_stats().map_err(Error::Milli)?;

            search_result.facet_distribution = Some(distribution);

            if !stats.is_empty() {
                let facet_stats: BTreeMap<String, FacetStats> = stats
                    .into_iter()
                    .map(|(name, (min, max))| (name, FacetStats { min, max }))
                    .collect();
                search_result.facet_stats = Some(facet_stats);
            }
        }

        Ok(search_result)
    }

    /// Simple search method that returns raw JSON documents.
    ///
    /// This is a convenience method for simple searches. For more control,
    /// use [`search`](Self::search) with a [`SearchQuery`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// let docs = index.search_simple("hello world")?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn search_simple(&self, query: &str) -> Result<Vec<Value>> {
        let search_query = SearchQuery::new(query);
        let result = self.search(&search_query)?;
        Ok(result.hits.into_iter().map(|h| h.document).collect())
    }

    /// Search using a raw embedding vector via the external vector store.
    ///
    /// Returns up to `limit` documents sorted by vector similarity.
    /// If no vector store is configured, returns an empty list.
    pub fn search_vectors(&self, vector: &[f32], limit: usize) -> Result<Vec<Value>> {
        if let Some(store) = &self.vector_store {
            let results = store
                .search(vector, limit, None)
                .map_err(|e| Error::Internal(e.to_string()))?;

            let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
            let ids: Vec<u32> = results.iter().map(|(id, _)| *id).collect();
            let documents = self.inner.documents(&rtxn, ids).map_err(Error::Milli)?;
            let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
            let displayed_fields = self
                .inner
                .displayed_fields_ids(&rtxn, &fields_ids_map)
                .map_err(Error::Milli)?
                .map(|fields| fields.into_iter().collect::<Vec<_>>())
                .unwrap_or_else(|| fields_ids_map.ids().collect());

            let mut docs = Vec::new();
            for (_id, obkv) in documents {
                let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                    .map_err(Error::Milli)?;
                docs.push(Value::Object(json));
            }
            Ok(docs)
        } else {
            Ok(Vec::new())
        }
    }
}

fn format_document(
    value: &mut Value,
    path: &str,
    indices: &mut Vec<usize>,
    matcher: &MatcherBuilder<'_>,
    query: &SearchQuery,
    locales: Option<&[milli::tokenizer::Language]>,
    matches: &mut BTreeMap<String, Vec<crate::core::MatchBounds>>,
) {
    match value {
        Value::Object(object) => {
            for (field, value) in object {
                if field == "_vectors" {
                    continue;
                }
                let path = if path.is_empty() {
                    field.clone()
                } else {
                    format!("{path}.{field}")
                };
                format_document(value, &path, indices, matcher, query, locales, matches);
            }
        }
        Value::Array(array) => {
            for (index, value) in array.iter_mut().enumerate() {
                indices.push(index);
                format_document(value, path, indices, matcher, query, locales, matches);
                indices.pop();
            }
        }
        Value::String(text) => {
            let selected = |selector: &str| {
                selector == "*"
                    || selector == path
                    || path
                        .strip_prefix(selector)
                        .is_some_and(|rest| rest.starts_with('.'))
            };
            let highlight = query
                .attributes_to_highlight
                .as_ref()
                .is_some_and(|fields| fields.iter().any(|field| selected(field)));
            let crop = query.attributes_to_crop.as_ref().and_then(|fields| {
                fields.iter().find_map(|field| {
                    let (field, length) = field
                        .split_once(':')
                        .map_or((field.as_str(), query.crop_length), |(f, l)| {
                            (f, l.parse().unwrap_or(query.crop_length))
                        });
                    selected(field).then_some(length.min(10_000))
                })
            });
            let mut m = matcher.build(text, locales);
            if query.show_matches_position {
                let bounds = m.matches(indices);
                if !bounds.is_empty() {
                    matches.entry(path.into()).or_default().extend(bounds);
                }
            }
            if highlight || crop.is_some() {
                *text = m.format(FormatOptions { highlight, crop }).into_owned();
            }
        }
        _ => {}
    }
}
