use milli::score_details::ScoreDetails;
use milli::update::InnerIndexSettings;
use milli::{Filter, SearchForFacetValues, Similar, TermsMatchingStrategy};
use serde_json::Value;
use std::time::Instant;

use crate::core::error::{Error, Result};
use crate::core::search::{
    FacetHit, FacetSearchQuery, FacetSearchResult, HitsInfo, MatchingStrategy, SearchHit,
    SimilarQuery, SimilarResult,
};

use super::{parse_filter_to_string, Index};

impl Index {
    /// Search within facet values.
    ///
    /// Given a facet name (and optionally a facet query and a search query),
    /// returns matching facet values with their document counts.
    pub fn facet_search(&self, query: &FacetSearchQuery) -> Result<FacetSearchResult> {
        let start_time = Instant::now();
        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;
        let progress = milli::progress::Progress::default();

        // Build the inner keyword search to scope facet results
        let mut inner_search = self.inner.search(&rtxn, &progress);

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

        // Apply filter (supports string, array-of-strings, and array-of-arrays)
        let facet_filter_owned;
        if let Some(ref filter_val) = query.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                facet_filter_owned = fs;
                if let Some(filter) = Filter::from_str(&facet_filter_owned).map_err(Error::Milli)? {
                    inner_search.filter(filter);
                }
            }
        }

        // Apply ranking score threshold
        if let Some(threshold) = query.ranking_score_threshold {
            inner_search.ranking_score_threshold(threshold);
        }

        // Apply attributes to search on
        let searchable_attrs_owned;
        if let Some(ref attrs) = query.attributes_to_search_on {
            searchable_attrs_owned = attrs.clone();
            inner_search.searchable_attributes(&searchable_attrs_owned);
        }

        // Build facet search
        let mut facet_search =
            SearchForFacetValues::new(query.facet_name.clone(), inner_search, false);

        if let Some(ref fq) = query.facet_query {
            facet_search.query(fq);
        }

        // Apply locales
        let parsed_locales = self.parse_locales(query.locales.as_deref())?;
        if let Some(locales) = parsed_locales {
            facet_search.locales(locales);
        }

        let facet_hits = facet_search.execute().map_err(Error::Milli)?;

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
        let start_time = Instant::now();

        let rtxn = self.inner.read_txn().map_err(|e| Error::Heed(e))?;

        // Resolve the embedder from index settings
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
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
        let progress = milli::progress::Progress::default();

        let mut similar = Similar::new(
            internal_id,
            query.offset,
            query.limit,
            &self.inner,
            &rtxn,
            query.embedder.clone(),
            embedder,
            quantized,
            &progress,
        );

        // Apply optional filter (supports string, array-of-strings, and array-of-arrays)
        let similar_filter_owned;
        if let Some(ref filter_val) = query.filter {
            if let Some(fs) = parse_filter_to_string(filter_val)? {
                similar_filter_owned = fs;
                if let Some(filter) = Filter::from_str(&similar_filter_owned).map_err(Error::Milli)? {
                    similar.filter(filter);
                }
            }
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

        // Fetch documents and build hits
        let documents = self
            .inner
            .documents(&rtxn, documents_ids.clone())
            .map_err(Error::Milli)?;
        let fields_ids_map = self.inner.fields_ids_map(&rtxn).map_err(Error::Heed)?;

        let displayed_fields = self.get_displayed_fields(
            &rtxn,
            &fields_ids_map,
            query.attributes_to_retrieve.as_ref(),
        )?;

        let mut hits = Vec::with_capacity(documents.len());
        for (idx, (_doc_id, obkv)) in documents.into_iter().enumerate() {
            let json = milli::obkv_to_json(&displayed_fields, &fields_ids_map, obkv)
                .map_err(Error::Milli)?;
            let doc = Value::Object(json);

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
                    .map(|scores| ScoreDetails::to_json_map(scores.iter()))
            } else {
                None
            };

            let mut hit = SearchHit::new(doc, ranking_score);
            hit.ranking_score_details = ranking_score_details;
            hits.push(hit);
        }

        let processing_time_ms = start_time.elapsed().as_millis();
        let total_hits = candidates.len() as usize;

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
