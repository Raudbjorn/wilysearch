use milli::Filter;
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::Instant;

use crate::core::error::{Error, Result};
use crate::core::search::{
    HybridSearchQuery, HybridSearchResult, SearchHit, SearchResult,
};

use super::{Index, parse_filter_to_string};

impl Index {
    /// Perform a hybrid search combining keyword and vector search.
    ///
    /// The `semantic_ratio` controls the balance between keyword and semantic search:
    /// - 0.0 = pure keyword search
    /// - 1.0 = pure semantic/vector search
    /// - 0.5 = balanced hybrid (default)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// # let embedding_vector = vec![0.1_f32; 384];
    /// use wilysearch::core::search::HybridSearchQuery;
    ///
    /// let query = HybridSearchQuery::new("search terms")
    ///     .with_vector(embedding_vector)
    ///     .with_semantic_ratio(0.7)  // Favor semantic search
    ///     .with_limit(10);
    ///
    /// let result = index.hybrid_search(&query)?;
    /// println!("Semantic hits: {:?}", result.result.semantic_hit_count);
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn hybrid_search(&self, query: &HybridSearchQuery) -> Result<HybridSearchResult> {
        let start_time = Instant::now();

        // If we have a vector store and a vector (or can embed the query), do hybrid search
        if let Some(store) = &self.vector_store {
            // Get the vector for semantic search
            let vector = match &query.vector {
                Some(v) => v.clone(),
                None => {
                    // If no vector provided, we'd need an embedder to generate one
                    // For now, fall back to keyword-only search
                    return self.hybrid_fallback_to_keyword(query, start_time);
                }
            };

            // Perform keyword search
            let keyword_result = self.search(&query.search)?;

            // Perform vector search
            let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;

            // Get filter candidates if filter is specified
            let filter_string_owned;
            let filter_bitmap = if let Some(filter_val) = &query.search.filter {
                if let Some(fs) = parse_filter_to_string(filter_val)? {
                    filter_string_owned = fs;
                    if let Some(filter) = Filter::from_str(&filter_string_owned).map_err(Error::Milli)? {
                        Some(filter.evaluate(&rtxn, &self.inner).map_err(Error::Milli)?)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let vector_results = store
                .search(&vector, query.search.limit + query.search.offset, filter_bitmap.as_ref())
                .map_err(|e| Error::Internal(e.to_string()))?;

            // Merge results based on semantic_ratio
            let merged = self.merge_hybrid_results(
                &rtxn,
                keyword_result.clone(),
                vector_results.clone(),
                query.semantic_ratio,
                query.search.limit,
                query.search.offset,
                query.search.show_ranking_score,
                query.search.attributes_to_retrieve.as_ref(),
            )?;

            let processing_time_ms = start_time.elapsed().as_millis();
            let query_string = query.search.q.clone().unwrap_or_default();

            // Count semantic hits
            let semantic_hit_count = merged.1;

            let keyword_estimated = match &keyword_result.hits_info {
                crate::core::search::HitsInfo::OffsetLimit { estimated_total_hits, .. } => *estimated_total_hits,
                crate::core::search::HitsInfo::Pagination { total_hits, .. } => *total_hits,
            };

            Ok(HybridSearchResult::new(
                SearchResult::new(
                    merged.0,
                    query_string,
                    processing_time_ms,
                    keyword_estimated.max(vector_results.len()),
                    query.search.limit,
                    query.search.offset,
                ),
                Some(semantic_hit_count),
            ))
        } else {
            // No vector store, fall back to keyword search
            self.hybrid_fallback_to_keyword(query, start_time)
        }
    }

    /// Fallback to keyword-only search when hybrid search isn't possible.
    fn hybrid_fallback_to_keyword(
        &self,
        query: &HybridSearchQuery,
        start_time: Instant,
    ) -> Result<HybridSearchResult> {
        let keyword_result = self.search(&query.search)?;
        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(HybridSearchResult::new(
            SearchResult {
                processing_time_ms,
                ..keyword_result
            },
            Some(0), // No semantic hits
        ))
    }

    /// Merge keyword and vector search results based on semantic ratio.
    #[allow(clippy::too_many_arguments)]
    fn merge_hybrid_results(
        &self,
        rtxn: &milli::heed::RoTxn<'_>,
        keyword_result: SearchResult,
        vector_results: Vec<(u32, f32)>,
        semantic_ratio: f32,
        limit: usize,
        offset: usize,
        show_ranking_score: bool,
        attributes_to_retrieve: Option<&BTreeSet<String>>,
    ) -> Result<(Vec<SearchHit>, u32)> {
        let fields_ids_map = self.inner.fields_ids_map(rtxn).map_err(Error::Heed)?;
        let displayed_fields = self.get_displayed_fields(rtxn, &fields_ids_map, attributes_to_retrieve)?;

        // Create scored entries for both result sets
        #[derive(Debug)]
        struct ScoredEntry {
            doc_id: u32,
            keyword_score: Option<f64>,
            #[allow(dead_code)]
            vector_score: Option<f32>,
            combined_score: f64,
            source: EntrySource,
        }

        #[derive(Debug, Clone, Copy)]
        enum EntrySource {
            Keyword,
            Vector,
            Both,
        }

        let keyword_weight = 1.0 - semantic_ratio as f64;
        let semantic_weight = semantic_ratio as f64;

        // Build a map of doc_id -> scores
        let mut score_map: std::collections::HashMap<u32, ScoredEntry> =
            std::collections::HashMap::new();

        // Add keyword results using the actual milli document IDs
        for (idx, hit) in keyword_result.hits.iter().enumerate() {
            let doc_id = keyword_result
                .document_ids
                .get(idx)
                .copied()
                .unwrap_or_else(|| {
                    tracing::warn!(
                        idx,
                        "keyword_result.document_ids missing entry; falling back to array index as doc_id"
                    );
                    idx as u32
                });

            let keyword_score = hit.ranking_score.unwrap_or(1.0);
            let combined = keyword_score * keyword_weight;

            score_map.insert(
                doc_id,
                ScoredEntry {
                    doc_id,
                    keyword_score: Some(keyword_score),
                    vector_score: None,
                    combined_score: combined,
                    source: EntrySource::Keyword,
                },
            );
        }

        // Add vector results
        let mut semantic_hit_count = 0u32;
        for (doc_id, similarity) in &vector_results {
            let vector_score = *similarity as f64;

            if let Some(entry) = score_map.get_mut(doc_id) {
                // Document found in both results
                entry.vector_score = Some(*similarity);
                entry.combined_score =
                    entry.keyword_score.unwrap_or(0.0) * keyword_weight + vector_score * semantic_weight;
                entry.source = EntrySource::Both;
            } else {
                // Document only in vector results
                score_map.insert(
                    *doc_id,
                    ScoredEntry {
                        doc_id: *doc_id,
                        keyword_score: None,
                        vector_score: Some(*similarity),
                        combined_score: vector_score * semantic_weight,
                        source: EntrySource::Vector,
                    },
                );
            }
        }

        // Sort by combined score (descending)
        let mut entries: Vec<_> = score_map.into_values().collect();
        entries.sort_by(|a, b| b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal));

        // Apply offset and limit
        let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

        // Fetch documents and build hits
        let doc_ids: Vec<u32> = entries.iter().map(|e| e.doc_id).collect();
        let documents = self.inner.documents(rtxn, doc_ids).map_err(Error::Milli)?;

        let mut hits = Vec::with_capacity(entries.len());
        let mut doc_map: std::collections::HashMap<u32, _> = documents.into_iter().collect();

        for entry in &entries {
            if let Some(obkv) = doc_map.remove(&entry.doc_id) {
                let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                    .map_err(Error::Milli)?;

                let ranking_score = if show_ranking_score {
                    Some(entry.combined_score)
                } else {
                    None
                };

                hits.push(SearchHit::new(Value::Object(json), ranking_score));

                // Count semantic hits (documents that came from vector search)
                if matches!(entry.source, EntrySource::Vector | EntrySource::Both) {
                    semantic_hit_count += 1;
                }
            }
        }

        Ok((hits, semantic_hit_count))
    }
}
