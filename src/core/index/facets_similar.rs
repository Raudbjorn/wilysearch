use milli::score_details::ScoreDetails;
use milli::update::InnerIndexSettings;
use milli::{SearchForFacetValues, Similar, TermsMatchingStrategy};
use std::time::Instant;

use crate::core::error::{Error, Result};
use crate::core::search::{
    FacetHit, FacetSearchQuery, FacetSearchResult, HitsInfo, MatchingStrategy, SearchHit,
    SimilarQuery, SimilarResult,
};

use super::Index;

impl Index {
    /// Search within facet values.
    ///
    /// Given a facet name (and optionally a facet query and a search query),
    /// returns matching facet values with their document counts.
    pub fn facet_search(&self, query: &FacetSearchQuery) -> Result<FacetSearchResult> {
        let filter = query
            .filter
            .as_ref()
            .map(super::parse_local_filter)
            .transpose()?
            .flatten();
        self.facet_search_with_filter(query, filter)
    }

    pub(crate) fn facet_search_with_filter(
        &self,
        query: &FacetSearchQuery,
        filter: Option<milli::IndexFilter>,
    ) -> Result<FacetSearchResult> {
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::quiet();
        let fields_ids_map = self.inner.fields_ids_map(&rtxn)?;

        // Build the inner keyword search to scope facet results
        let mut inner_search = self.inner.search(
            &rtxn,
            &self.uid,
            &fields_ids_map,
            time::OffsetDateTime::now_utc(),
            &progress,
        );

        if let Some(ref q) = query.q {
            inner_search.query(q);
        }

        // Apply matching strategy
        let tms = match query.matching_strategy {
            MatchingStrategy::Last => TermsMatchingStrategy::Last,
            MatchingStrategy::All => TermsMatchingStrategy::All,
            MatchingStrategy::Frequency => TermsMatchingStrategy::Frequency,
        };
        inner_search.terms_matching_strategy(tms);

        inner_search.filter(filter);
        inner_search.deadline(self.inner.search_deadline(&rtxn)?);
        // Apply ranking score threshold
        if let Some(threshold) = query.ranking_score_threshold {
            inner_search.ranking_score_threshold(threshold);
            inner_search.scoring_strategy(milli::score_details::ScoringStrategy::Detailed);
        }

        // Apply attributes to search on
        let searchable_attrs_owned;
        if let Some(ref attrs) = query.attributes_to_search_on {
            searchable_attrs_owned = attrs.clone();
            inner_search.searchable_attributes(&searchable_attrs_owned);
        }

        // Build facet search
        let mut facet_search = SearchForFacetValues::new(
            query.facet_name.clone(),
            &self.inner,
            &rtxn,
            &fields_ids_map,
        );

        if let Some(ref fq) = query.facet_query {
            facet_search.query(fq);
        }

        // Apply locales
        let parsed_locales = self.parse_locales(query.locales.as_deref())?;
        if let Some(locales) = parsed_locales {
            facet_search.locales(locales);
        }

        let facet_hits = facet_search
            .execute(&inner_search.execute_for_candidates(false)?)
            .map_err(Error::Milli)?
            .0;

        let processing_time_ms = start_time.elapsed().as_millis();

        Ok(FacetSearchResult {
            facet_hits: facet_hits
                .into_iter()
                .map(|fv| FacetHit {
                    value: fv.value,
                    count: fv.count,
                })
                .collect(),
            facet_query: query.facet_query.clone(),
            processing_time_ms,
        })
    }

    /// Find documents similar to a given document using vector embeddings.
    ///
    /// This method uses the configured embedder to look up the source document's
    /// vector and find nearby documents in embedding space. The source document
    /// is automatically excluded from the results.
    ///
    /// # Arguments
    ///
    /// * `query` - A [`SimilarQuery`] specifying the source document ID, embedder
    ///   name, pagination, optional filter, and score display options.
    ///
    /// # Errors
    ///
    /// * [`Error::EmbedderNotFound`] if the embedder named in the query is not
    ///   configured on this index.
    /// * [`Error::DocumentNotFound`] if the source document ID does not exist.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::search::SimilarQuery;
    /// use serde_json::json;
    ///
    /// let query = SimilarQuery {
    ///     id: json!("doc-42"),
    ///     embedder: "default".to_string(),
    ///     offset: 0,
    ///     limit: 10,
    ///     filter: None,
    ///     attributes_to_retrieve: None,
    ///     retrieve_vectors: false,
    ///     show_ranking_score: true,
    ///     show_ranking_score_details: false,
    ///     ranking_score_threshold: None,
    /// };
    ///
    /// let result = index.get_similar_documents(&query)?;
    /// for hit in &result.hits {
    ///     println!("{}", hit.document);
    /// }
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn get_similar_documents(&self, query: &SimilarQuery) -> Result<SimilarResult> {
        let filter = query
            .filter
            .as_ref()
            .map(super::parse_local_filter)
            .transpose()?
            .flatten();
        self.similar_with_filter(query, filter)
    }

    pub(crate) fn similar_with_filter(
        &self,
        query: &SimilarQuery,
        filter: Option<milli::IndexFilter>,
    ) -> Result<SimilarResult> {
        let start_time = Instant::now();

        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;

        // Resolve the embedder from index settings
        let ip_policy = self.ip_policy.clone();
        let inner_settings = InnerIndexSettings::from_index(&self.inner, &rtxn, &ip_policy, None)
            .map_err(Error::Milli)?;

        let runtime_embedder = inner_settings
            .runtime_embedders
            .get(&query.embedder)
            .ok_or_else(|| Error::EmbedderNotFound(query.embedder.clone()))?;

        let embedder = runtime_embedder.embedder.clone();
        let quantized = runtime_embedder.is_quantized;

        // Convert the JSON document ID to a string, then resolve to an internal ID
        let id_string = milli::documents::validate_document_id_value(query.id.clone())
            .map_err(|e| Error::Internal(format!("Invalid document id: {e}")))?;

        let external_ids = self.inner.external_documents_ids();
        let internal_id = external_ids
            .get(&rtxn, &id_string)
            .map_err(Error::Heed)?
            .ok_or_else(|| Error::DocumentNotFound(id_string.clone()))?;

        // Build the Similar query
        let progress = milli::progress::Progress::quiet();
        let fields_ids_map = self.inner.fields_ids_map(&rtxn)?;
        let max_hits = self.inner.pagination_max_total_hits(&rtxn)?.unwrap_or(1000) as usize;
        let offset = query.offset.min(max_hits);
        let limit = query.limit.min(max_hits.saturating_sub(offset));

        let mut similar = Similar::new(
            internal_id,
            offset,
            limit,
            &self.inner,
            &rtxn,
            &fields_ids_map,
            query.embedder.clone(),
            embedder,
            quantized,
            &progress,
        );

        if let Some(filter) = filter {
            similar.filter(filter);
        }
        // Apply ranking score threshold
        if let Some(threshold) = query.ranking_score_threshold {
            similar.ranking_score_threshold(threshold);
        }

        // Execute the similar search
        let milli::SearchResult {
            documents_ids,
            candidates,
            document_scores,
            ..
        } = similar.execute().map_err(Error::Milli)?;

        let criteria = milli::AttributeState::from_criteria(self.inner.criteria(&rtxn)?);
        let requested = query
            .attributes_to_retrieve
            .as_ref()
            .map(|a| a.iter().cloned().collect::<Vec<_>>());
        let mut hits = Vec::with_capacity(documents_ids.len());
        for (idx, id) in documents_ids.into_iter().enumerate() {
            let doc = self.make_document(
                &rtxn,
                &fields_ids_map,
                id,
                requested.as_deref(),
                query.retrieve_vectors,
                true,
            )?;
            let ranking_score = if query.show_ranking_score {
                document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::global_score(scores.iter()))
            } else {
                None
            };

            let ranking_score_details = if query.show_ranking_score_details {
                document_scores
                    .get(idx)
                    .map(|scores| ScoreDetails::to_json_map(criteria, scores.iter()))
            } else {
                None
            };

            let mut hit = SearchHit::new(doc, ranking_score);
            hit.ranking_score_details = ranking_score_details;
            hits.push(hit);
        }

        let processing_time_ms = start_time.elapsed().as_millis();
        let total_hits = (candidates.len() as usize).min(max_hits);

        Ok(SimilarResult {
            hits,
            id: id_string,
            processing_time_ms,
            hits_info: HitsInfo::OffsetLimit {
                limit: query.limit,
                offset: query.offset,
                estimated_total_hits: total_hits,
            },
        })
    }
}
