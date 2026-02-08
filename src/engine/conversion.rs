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
    if req.hybrid.is_some() {
        tracing::warn!(
            "SearchRequest.hybrid is set but not yet wired through to the core search engine; \
             hybrid search requires the RAG pipeline or direct vector API"
        );
    }
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
            .map(|(k, v)| (k.clone(), v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect()))
            .collect()
    });
    let facet_stats = r
        .facet_stats
        .as_ref()
        .map(|fs| serde_json::to_value(fs))
        .transpose()?;

    Ok(SearchResponse {
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

pub(super) fn convert_federation_settings(s: &FederationSettings) -> crate::core::search::Federation {
    crate::core::search::Federation {
        limit: s.limit.unwrap_or(20) as usize,
        offset: s.offset.unwrap_or(0) as usize,
        page: s.page.map(|p| p as usize),
        hits_per_page: s.hits_per_page.map(|h| h as usize),
        facets_by_index: s.facets_by_index.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        merge_facets: s.merge_facets.map(|mf| crate::core::search::MergeFacets {
            max_values_per_facet: mf.max_values_per_facet.map(|v| v as usize),
        }),
    }
}

pub(super) fn convert_federated_result(r: &crate::core::search::FederatedSearchResult) -> Result<FederatedSearchResponse> {
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
            .map(|(k, v)| (k.clone(), v.iter().map(|(k2, v2)| (k2.clone(), *v2)).collect()))
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

pub(super) fn convert_settings_to_lib(s: &Settings) -> Result<crate::core::Settings> {
    let mut ms = crate::core::Settings::new();
    if let Some(ref rules) = s.ranking_rules {
        ms = ms.with_ranking_rules(rules.clone());
    }
    if let Some(ref attr) = s.distinct_attribute {
        ms = ms.with_distinct_attribute(attr.clone());
    }
    if let Some(ref attrs) = s.searchable_attributes {
        ms = ms.with_searchable_attributes(attrs.clone());
    }
    if let Some(ref attrs) = s.displayed_attributes {
        ms = ms.with_displayed_attributes(attrs.clone());
    }
    if let Some(ref words) = s.stop_words {
        ms = ms.with_stop_words(words.iter().cloned().collect());
    }
    if let Some(ref syns) = s.synonyms {
        ms = ms.with_synonyms(syns.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }
    if let Some(ref attrs) = s.filterable_attributes {
        ms = ms.with_filterable_attributes(attrs.clone());
    }
    if let Some(ref attrs) = s.sortable_attributes {
        ms = ms.with_sortable_attributes(attrs.iter().cloned().collect());
    }
    if let Some(ref typo) = s.typo_tolerance {
        let lib_typo = crate::core::settings::TypoToleranceSettings {
            enabled: typo.enabled,
            min_word_size_for_typos: typo.min_word_size_for_typos.as_ref().map(|m| {
                crate::core::settings::MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| u8::try_from(v).unwrap_or_else(|_| {
                        tracing::warn!(value = v, "one_typo value {} exceeds u8::MAX, clamping to 255", v);
                        u8::MAX
                    })),
                    two_typos: m.two_typos.map(|v| u8::try_from(v).unwrap_or_else(|_| {
                        tracing::warn!(value = v, "two_typos value {} exceeds u8::MAX, clamping to 255", v);
                        u8::MAX
                    })),
                }
            }),
            disable_on_words: typo.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: typo.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: typo.disable_on_numbers,
        };
        ms = ms.with_typo_tolerance(lib_typo);
    }
    if let Some(ref pag) = s.pagination {
        let lib_pag = crate::core::settings::PaginationSettings {
            max_total_hits: pag.max_total_hits.map(|v| v as usize),
        };
        ms = ms.with_pagination(lib_pag);
    }
    if let Some(ref fac) = s.faceting {
        let sort_map = if let Some(ref m) = fac.sort_facet_values_by {
            let mut result = std::collections::BTreeMap::new();
            for (k, v) in m {
                let sort = match v {
                    FacetValuesSort::Alpha => crate::core::settings::FacetValuesSort::Alpha,
                    FacetValuesSort::Count => crate::core::settings::FacetValuesSort::Count,
                };
                result.insert(k.clone(), sort);
            }
            Some(result)
        } else {
            None
        };
        let lib_fac = crate::core::settings::FacetingSettings {
            max_values_per_facet: fac.max_values_per_facet.map(|v| v as usize),
            sort_facet_values_by: sort_map,
        };
        ms = ms.with_faceting(lib_fac);
    }
    if let Some(ref dict) = s.dictionary {
        ms = ms.with_dictionary(dict.iter().cloned().collect());
    }
    if let Some(ref tokens) = s.separator_tokens {
        ms = ms.with_separator_tokens(tokens.iter().cloned().collect());
    }
    if let Some(ref tokens) = s.non_separator_tokens {
        ms = ms.with_non_separator_tokens(tokens.iter().cloned().collect());
    }
    if let Some(ref prec) = s.proximity_precision {
        let lib_prec = match prec {
            ProximityPrecision::ByWord => crate::core::settings::ProximityPrecision::ByWord,
            ProximityPrecision::ByAttribute => crate::core::settings::ProximityPrecision::ByAttribute,
        };
        ms = ms.with_proximity_precision(lib_prec);
    }
    if let Some(enabled) = s.facet_search {
        ms = ms.with_facet_search(enabled);
    }
    if let Some(ref mode) = s.prefix_search {
        let mode_str = match mode {
            PrefixSearch::IndexingTime => "indexingTime".to_string(),
            PrefixSearch::Disabled => "disabled".to_string(),
        };
        ms = ms.with_prefix_search(mode_str);
    }
    if let Some(ms_val) = s.search_cutoff_ms {
        ms = ms.with_search_cutoff_ms(ms_val);
    }
    if let Some(ref rules) = s.localized_attributes {
        let lib_rules: Vec<crate::core::settings::LocalizedAttributeRule> = rules
            .iter()
            .map(|r| crate::core::settings::LocalizedAttributeRule {
                attribute_patterns: r.attribute_patterns.clone(),
                locales: r.locales.clone(),
            })
            .collect();
        ms = ms.with_localized_attributes(lib_rules);
    }
    if let Some(ref embs) = s.embedders {
        let lib_embs: HashMap<String, crate::core::EmbedderSettings> = embs
            .iter()
            .map(|(k, v)| {
                let mut es = crate::core::EmbedderSettings::default();
                let source = parse_embedder_source(&v.source);
                es.source = Some(source);
                es.api_key = v.api_key.clone();
                es.model = v.model.clone();
                es.dimensions = v.dimensions.map(|d| d as usize);
                es.url = v.url.clone();
                (k.clone(), es)
            })
            .collect();
        ms = ms.with_embedders(lib_embs);
    }
    Ok(ms)
}

/// Parse an embedder source string, accepting both camelCase and lowercase variants.
pub(super) fn parse_embedder_source(s: &str) -> crate::core::EmbedderSource {
    match s {
        "openai" | "openAi" => crate::core::EmbedderSource::OpenAi,
        "huggingface" | "huggingFace" => crate::core::EmbedderSource::HuggingFace,
        "ollama" => crate::core::EmbedderSource::Ollama,
        "userprovided" | "userProvided" => crate::core::EmbedderSource::UserProvided,
        "rest" => crate::core::EmbedderSource::Rest,
        "composite" => crate::core::EmbedderSource::Composite,
        _ => {
            tracing::warn!(
                source = s,
                "unknown embedder source '{}', falling back to default",
                s
            );
            crate::core::EmbedderSource::default()
        }
    }
}

/// Convert an EmbedderSource to its canonical camelCase string.
pub(super) fn embedder_source_to_str(s: &crate::core::EmbedderSource) -> &'static str {
    match s {
        crate::core::EmbedderSource::OpenAi => "openAi",
        crate::core::EmbedderSource::HuggingFace => "huggingFace",
        crate::core::EmbedderSource::Ollama => "ollama",
        crate::core::EmbedderSource::UserProvided => "userProvided",
        crate::core::EmbedderSource::Rest => "rest",
        crate::core::EmbedderSource::Composite => "composite",
    }
}

pub(super) fn convert_settings_from_lib(s: &crate::core::Settings) -> Settings {
    Settings {
        ranking_rules: s.ranking_rules.clone(),
        distinct_attribute: s.distinct_attribute.clone(),
        searchable_attributes: s.searchable_attributes.clone(),
        displayed_attributes: s.displayed_attributes.clone(),
        stop_words: s.stop_words.as_ref().map(|w| w.iter().cloned().collect()),
        synonyms: s.synonyms.as_ref().map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        }),
        filterable_attributes: s.filterable_attributes.clone(),
        sortable_attributes: s.sortable_attributes.as_ref().map(|a| a.iter().cloned().collect()),
        typo_tolerance: s.typo_tolerance.as_ref().map(|t| TypoTolerance {
            enabled: t.enabled,
            min_word_size_for_typos: t.min_word_size_for_typos.as_ref().map(|m| {
                MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| v as u32),
                    two_typos: m.two_typos.map(|v| v as u32),
                }
            }),
            disable_on_words: t.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: t.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: t.disable_on_numbers,
        }),
        pagination: s.pagination.as_ref().map(|p| Pagination {
            max_total_hits: p.max_total_hits.map(usize_to_u64),
        }),
        faceting: s.faceting.as_ref().map(|f| Faceting {
            max_values_per_facet: f.max_values_per_facet.map(usize_to_u64),
            sort_facet_values_by: f.sort_facet_values_by.as_ref().map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let s = match v {
                            crate::core::settings::FacetValuesSort::Count => FacetValuesSort::Count,
                            crate::core::settings::FacetValuesSort::Alpha => FacetValuesSort::Alpha,
                        };
                        (k.clone(), s)
                    })
                    .collect()
            }),
        }),
        dictionary: s.dictionary.as_ref().map(|d| d.iter().cloned().collect()),
        separator_tokens: s.separator_tokens.as_ref().map(|t| t.iter().cloned().collect()),
        non_separator_tokens: s.non_separator_tokens.as_ref().map(|t| t.iter().cloned().collect()),
        proximity_precision: s.proximity_precision.as_ref().map(|p| match p {
            crate::core::settings::ProximityPrecision::ByWord => ProximityPrecision::ByWord,
            crate::core::settings::ProximityPrecision::ByAttribute => ProximityPrecision::ByAttribute,
        }),
        facet_search: s.facet_search,
        prefix_search: s.prefix_search.as_ref().map(|s| match s.as_str() {
            "disabled" => PrefixSearch::Disabled,
            _ => PrefixSearch::IndexingTime,
        }),
        search_cutoff_ms: s.search_cutoff_ms,
        localized_attributes: s.localized_attributes.as_ref().map(|rules| {
            rules
                .iter()
                .map(|r| LocalizedAttribute {
                    attribute_patterns: r.attribute_patterns.clone(),
                    locales: r.locales.clone(),
                })
                .collect()
        }),
        embedders: s.embedders.as_ref().map(|embs| {
            embs.iter()
                .map(|(k, v)| {
                    let source = v
                        .source
                        .as_ref()
                        .map(|s| embedder_source_to_str(s).to_string())
                        .unwrap_or_default();
                    (
                        k.clone(),
                        EmbedderConfig {
                            source,
                            api_key: v.api_key.clone(),
                            model: v.model.clone(),
                            dimensions: v.dimensions.map(saturating_u32),
                            url: v.url.clone(),
                            extra: HashMap::new(),
                        },
                    )
                })
                .collect()
        }),
    }
}
