use milli::score_details::{ScoreDetails, ScoringStrategy};
use milli::{AscDesc, Filter, FacetDistribution, OrderBy, TermsMatchingStrategy};
use milli::tokenizer::TokenizerBuilder;
use milli::{MatcherBuilder, FormatOptions};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Instant;
use tracing::instrument;

use crate::core::error::{Error, Result};
use crate::core::search::{
    FacetStats, MatchingStrategy, SearchHit, SearchQuery, SearchResult,
};

use super::{Index, parse_filter_to_string};

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
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::default();

        let mut search = self.inner.search(&rtxn, &progress);

        // Set query string if provided
        if let Some(q) = &query.q {
            search.query(q);
        }

        // ------------------------------------------------------------------
        // Pagination: page-based vs offset/limit
        // ------------------------------------------------------------------
        let use_page_pagination = query.page.is_some() || query.hits_per_page.is_some();
        let (effective_offset, effective_limit, page_val, hpp_val) = if use_page_pagination {
            let page = query.page.unwrap_or(1);
            let hits_per_page = query.hits_per_page.unwrap_or(20);
            let computed_offset = page.saturating_sub(1) * hits_per_page;
            (computed_offset, hits_per_page, page, hits_per_page)
        } else {
            (query.offset, query.limit, 0, 0)
        };

        search.offset(effective_offset);
        search.limit(effective_limit);

        // For page-based pagination we need exhaustive counts
        if use_page_pagination {
            search.exhaustive_number_hits(true);
        }

        // ------------------------------------------------------------------
        // Scoring strategy
        // ------------------------------------------------------------------
        if query.show_ranking_score || query.show_ranking_score_details {
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
        let filter_owned;
        if let Some(filter_val) = &query.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                filter_owned = fs;
                if let Some(filter) = Filter::from_str(&filter_owned).map_err(Error::Milli)? {
                    search.filter(filter);
                }
            }
        }

        // ------------------------------------------------------------------
        // Sort criteria
        // ------------------------------------------------------------------
        if let Some(sort_strings) = &query.sort {
            let mut criteria = Vec::with_capacity(sort_strings.len());
            for s in sort_strings {
                let asc_desc = AscDesc::from_str(s)
                    .map_err(|e| Error::InvalidSort(format!("{e}")))?;
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
        let result = search.execute().map_err(Error::Milli)?;

        // Get the documents -- preserve milli IDs for hybrid search merging
        let returned_doc_ids = result.documents_ids.clone();
        let documents = self.inner.documents(&rtxn, result.documents_ids.clone()).map_err(Error::Milli)?;
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        // Determine which fields to display
        let displayed_fields = self.get_displayed_fields(
            &rtxn,
            &fields_ids_map,
            query.attributes_to_retrieve.as_ref(),
        )?;

        // ------------------------------------------------------------------
        // Highlighting / formatting / matches_position setup
        // ------------------------------------------------------------------
        let needs_formatting = query.attributes_to_highlight.is_some()
            || query.attributes_to_crop.is_some();
        let needs_matches = query.show_matches_position;
        let needs_matcher = needs_formatting || needs_matches;

        // Build MatcherBuilder if needed (consumes matching_words from result)
        let tokenizer;
        let mut matcher_builder_storage;
        let matcher_builder_opt: Option<&MatcherBuilder<'_>> = if needs_matcher {
            tokenizer = TokenizerBuilder::default().into_tokenizer();
            matcher_builder_storage = MatcherBuilder::new(result.matching_words, tokenizer);
            matcher_builder_storage.crop_marker(query.crop_marker.clone());
            matcher_builder_storage.highlight_prefix(query.highlight_pre_tag.clone());
            matcher_builder_storage.highlight_suffix(query.highlight_post_tag.clone());
            Some(&matcher_builder_storage)
        } else {
            None
        };

        // Compute which fields should be highlighted/cropped
        let highlight_fields: BTreeSet<String> = query
            .attributes_to_highlight
            .as_ref()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        let crop_fields: Vec<String> = query
            .attributes_to_crop
            .clone()
            .unwrap_or_default();

        // ------------------------------------------------------------------
        // Build hits
        // ------------------------------------------------------------------
        let mut hits = Vec::with_capacity(documents.len());
        for (idx, (_id, obkv)) in documents.into_iter().enumerate() {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            let doc = Value::Object(json.clone());

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
                    .map(|scores| ScoreDetails::to_json_map(scores.iter()))
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

                for (field_name, value) in &json {
                    if let Value::String(text) = value {
                        let should_highlight = highlight_fields.contains("*")
                            || highlight_fields.contains(field_name);
                        // Cap crop length at 10_000 words to prevent degenerate
                        // allocations from absurd user-supplied values.
                        const MAX_CROP_LENGTH: usize = 10_000;
                        let crop_len = crop_fields.iter().find_map(|c| {
                            let parts: Vec<&str> = c.splitn(2, ':').collect();
                            if parts[0] == "*" || parts[0] == field_name {
                                if parts.len() > 1 {
                                    parts[1].parse::<usize>().ok().map(|v| v.min(MAX_CROP_LENGTH))
                                } else {
                                    Some(query.crop_length.min(MAX_CROP_LENGTH))
                                }
                            } else {
                                None
                            }
                        });

                        let format_options = FormatOptions {
                            highlight: should_highlight,
                            crop: crop_len,
                        };

                        if format_options.should_format() {
                            let mut matcher = mb.build(
                                text,
                                parsed_locales.as_deref(),
                            );
                            let formatted_text = matcher.format(format_options);
                            formatted_doc.insert(
                                field_name.clone(),
                                Value::String(formatted_text.into_owned()),
                            );
                        }

                        if needs_matches {
                            let mut matcher = mb.build(
                                text,
                                parsed_locales.as_deref(),
                            );
                            let bounds = matcher.matches(&[]);
                            if !bounds.is_empty() {
                                let converted: Vec<crate::core::search::MatchBounds> = bounds
                                    .into_iter()
                                    .map(|b| crate::core::search::MatchBounds {
                                        start: b.start,
                                        length: b.length,
                                    })
                                    .collect();
                                all_matches.insert(field_name.clone(), converted);
                            }
                        }
                    }
                }

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
        let total_hits = result.candidates.len() as usize;
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
        if let Some(ref facet_names) = query.facets {
            let mut fd = FacetDistribution::new(&rtxn, &self.inner);
            let facets_with_order: Vec<(String, OrderBy)> = facet_names
                .iter()
                .map(|name| (name.clone(), OrderBy::Lexicographic))
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
            let results =
                store.search(vector, limit, None).map_err(|e| Error::Internal(e.to_string()))?;

            let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
            let ids: Vec<u32> = results.iter().map(|(id, _)| *id).collect();
            let documents = self.inner.documents(&rtxn, ids).map_err(Error::Milli)?;
            let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;
            let displayed_fields = self
                .inner
                .displayed_fields_ids(&rtxn)
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
