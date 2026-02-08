//! `SettingsApi` trait implementation for `Engine`.

use std::collections::{BTreeMap, HashMap};

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;
use super::conversion::*;
use super::{saturating_u32, usize_to_u64};

impl traits::SettingsApi for Engine {
    fn get_settings(&self, index_uid: &str) -> Result<Settings> {
        let idx = self.resolve_index(index_uid)?;
        let lib_settings = idx.get_settings()?;
        Ok(convert_settings_from_lib(&lib_settings))
    }

    fn update_settings(&self, index_uid: &str, settings: &Settings) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_settings = convert_settings_to_lib(settings)?;
        idx.update_settings(&lib_settings)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn reset_settings(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_settings()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    // ── Sub-settings ─────────────────────────────────────────────────────

    fn get_ranking_rules(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_ranking_rules()?.unwrap_or_default())
    }
    fn update_ranking_rules(&self, index_uid: &str, rules: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_ranking_rules(rules.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_ranking_rules(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_ranking_rules()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_distinct_attribute(&self, index_uid: &str) -> Result<Option<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_distinct_attribute()?)
    }
    fn update_distinct_attribute(&self, index_uid: &str, attr: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_distinct_attribute(attr.to_string())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_distinct_attribute(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_distinct_attribute()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_searchable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_searchable_attributes()?.unwrap_or_default())
    }
    fn update_searchable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_searchable_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_searchable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_searchable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_displayed_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_displayed_attributes()?.unwrap_or_default())
    }
    fn update_displayed_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_displayed_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_displayed_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_displayed_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_synonyms(&self, index_uid: &str) -> Result<HashMap<String, Vec<String>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_synonyms()?
            .map(|m| m.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_synonyms(
        &self,
        index_uid: &str,
        synonyms: &HashMap<String, Vec<String>>,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let btree: BTreeMap<String, Vec<String>> = synonyms.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        idx.update_synonyms(btree)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_synonyms(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_synonyms()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_stop_words(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_stop_words()?
            .map(|s| s.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_stop_words(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_stop_words(words.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_stop_words(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_stop_words()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_filterable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_filterable_attributes()?.unwrap_or_default())
    }
    fn update_filterable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_filterable_attributes(attrs.to_vec())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_filterable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_filterable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_sortable_attributes(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_sortable_attributes()?
            .map(|s| s.into_iter().collect())
            .unwrap_or_default())
    }
    fn update_sortable_attributes(&self, index_uid: &str, attrs: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_sortable_attributes(attrs.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_sortable_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_sortable_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_typo_tolerance(&self, index_uid: &str) -> Result<TypoTolerance> {
        let idx = self.resolve_index(index_uid)?;
        let lib = idx.get_typo_tolerance()?;
        Ok(lib
            .map(|t| TypoTolerance {
                enabled: t.enabled,
                min_word_size_for_typos: t.min_word_size_for_typos.map(|m| MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| v as u32),
                    two_typos: m.two_typos.map(|v| v as u32),
                }),
                disable_on_words: t.disable_on_words.map(|w| w.into_iter().collect()),
                disable_on_attributes: t.disable_on_attributes.map(|a| a.into_iter().collect()),
                disable_on_numbers: t.disable_on_numbers,
            })
            .unwrap_or_default())
    }
    fn update_typo_tolerance(&self, index_uid: &str, config: &TypoTolerance) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_typo = crate::core::settings::TypoToleranceSettings {
            enabled: config.enabled,
            min_word_size_for_typos: config.min_word_size_for_typos.as_ref().map(|m| {
                crate::core::settings::MinWordSizeForTypos {
                    one_typo: m.one_typo.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                    two_typos: m.two_typos.map(|v| u8::try_from(v).unwrap_or(u8::MAX)),
                }
            }),
            disable_on_words: config.disable_on_words.as_ref().map(|w| w.iter().cloned().collect()),
            disable_on_attributes: config.disable_on_attributes.as_ref().map(|a| a.iter().cloned().collect()),
            disable_on_numbers: config.disable_on_numbers,
        };
        idx.update_typo_tolerance(lib_typo)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_typo_tolerance(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_typo_tolerance()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_pagination(&self, index_uid: &str) -> Result<Pagination> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_pagination()?
            .map(|p| Pagination {
                max_total_hits: p.max_total_hits.map(usize_to_u64),
            })
            .unwrap_or_default())
    }
    fn update_pagination(&self, index_uid: &str, config: &Pagination) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_pagination(crate::core::settings::PaginationSettings {
            max_total_hits: config.max_total_hits.map(|v| v as usize),
        })?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_pagination(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_pagination()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_faceting(&self, index_uid: &str) -> Result<Faceting> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_faceting()?
            .map(|f| Faceting {
                max_values_per_facet: f.max_values_per_facet.map(usize_to_u64),
                sort_facet_values_by: f.sort_facet_values_by.map(|m| {
                    m.into_iter()
                        .map(|(k, v)| {
                            let s = match v {
                                crate::core::settings::FacetValuesSort::Count => FacetValuesSort::Count,
                                crate::core::settings::FacetValuesSort::Alpha => FacetValuesSort::Alpha,
                            };
                            (k, s)
                        })
                        .collect()
                }),
            })
            .unwrap_or_default())
    }
    fn update_faceting(&self, index_uid: &str, config: &Faceting) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let sort_map = if let Some(m) = config.sort_facet_values_by.as_ref() {
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
        idx.update_faceting(crate::core::settings::FacetingSettings {
            max_values_per_facet: config.max_values_per_facet.map(|v| v as usize),
            sort_facet_values_by: sort_map,
        })?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_faceting(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_faceting()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_dictionary(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_dictionary()?.map(|d| d.into_iter().collect()).unwrap_or_default())
    }
    fn update_dictionary(&self, index_uid: &str, words: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_dictionary(words.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_dictionary(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_dictionary()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_separator_tokens()?.map(|t| t.into_iter().collect()).unwrap_or_default())
    }
    fn update_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_separator_tokens(tokens.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_separator_tokens()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_non_separator_tokens(&self, index_uid: &str) -> Result<Vec<String>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_non_separator_tokens()?.map(|t| t.into_iter().collect()).unwrap_or_default())
    }
    fn update_non_separator_tokens(&self, index_uid: &str, tokens: &[String]) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_non_separator_tokens(tokens.iter().cloned().collect())?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_non_separator_tokens(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_non_separator_tokens()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_proximity_precision(&self, index_uid: &str) -> Result<ProximityPrecision> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_proximity_precision()?
            .map(|p| match p {
                crate::core::settings::ProximityPrecision::ByWord => ProximityPrecision::ByWord,
                crate::core::settings::ProximityPrecision::ByAttribute => ProximityPrecision::ByAttribute,
            })
            .unwrap_or(ProximityPrecision::ByWord))
    }
    fn update_proximity_precision(&self, index_uid: &str, precision: ProximityPrecision) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let p = match precision {
            ProximityPrecision::ByWord => crate::core::settings::ProximityPrecision::ByWord,
            ProximityPrecision::ByAttribute => crate::core::settings::ProximityPrecision::ByAttribute,
        };
        idx.update_proximity_precision(p)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_proximity_precision(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_proximity_precision()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_facet_search(&self, index_uid: &str) -> Result<bool> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_facet_search()?.unwrap_or(true))
    }
    fn update_facet_search(&self, index_uid: &str, enabled: bool) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_facet_search(enabled)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_facet_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_facet_search()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_prefix_search(&self, index_uid: &str) -> Result<PrefixSearch> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx
            .get_prefix_search()?
            .map(|s| match s.as_str() {
                "disabled" => PrefixSearch::Disabled,
                _ => PrefixSearch::IndexingTime,
            })
            .unwrap_or(PrefixSearch::IndexingTime))
    }
    fn update_prefix_search(&self, index_uid: &str, mode: PrefixSearch) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let mode_str = match mode {
            PrefixSearch::IndexingTime => "indexingTime".to_string(),
            PrefixSearch::Disabled => "disabled".to_string(),
        };
        idx.update_prefix_search(mode_str)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_prefix_search(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_prefix_search()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_search_cutoff_ms(&self, index_uid: &str) -> Result<Option<u64>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_search_cutoff_ms()?)
    }
    fn update_search_cutoff_ms(&self, index_uid: &str, ms: u64) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_search_cutoff_ms(ms)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_search_cutoff_ms(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_search_cutoff_ms()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_localized_attributes(
        &self,
        index_uid: &str,
    ) -> Result<Option<Vec<LocalizedAttribute>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_localized_attributes()?.map(|rules| {
            rules
                .into_iter()
                .map(|r| LocalizedAttribute {
                    locales: r.locales,
                    attribute_patterns: r.attribute_patterns,
                })
                .collect()
        }))
    }
    fn update_localized_attributes(
        &self,
        index_uid: &str,
        attrs: &[LocalizedAttribute],
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_rules: Vec<crate::core::settings::LocalizedAttributeRule> = attrs
            .iter()
            .map(|a| crate::core::settings::LocalizedAttributeRule {
                attribute_patterns: a.attribute_patterns.clone(),
                locales: a.locales.clone(),
            })
            .collect();
        idx.update_localized_attributes(lib_rules)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_localized_attributes(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_localized_attributes()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }

    fn get_embedders(
        &self,
        index_uid: &str,
    ) -> Result<Option<HashMap<String, EmbedderConfig>>> {
        let idx = self.resolve_index(index_uid)?;
        Ok(idx.get_embedders()?.map(|embs| {
            embs.into_iter()
                .map(|(k, v)| {
                    let source = v
                        .source
                        .as_ref()
                        .map(|s| embedder_source_to_str(s).to_string())
                        .unwrap_or_default();
                    (
                        k,
                        EmbedderConfig {
                            source,
                            api_key: v.api_key,
                            model: v.model,
                            dimensions: v.dimensions.map(saturating_u32),
                            url: v.url,
                            extra: HashMap::new(),
                        },
                    )
                })
                .collect()
        }))
    }
    fn update_embedders(
        &self,
        index_uid: &str,
        embedders: &HashMap<String, EmbedderConfig>,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let lib_embs: HashMap<String, crate::core::EmbedderSettings> = embedders
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
        idx.update_embedders(lib_embs)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_embedders(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.reset_embedders()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
}
