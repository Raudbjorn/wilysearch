use super::Index;
use crate::core::{Result, Settings};
use crate::types::*;

impl Index {
    pub fn get_settings(&self) -> Result<Settings> {
        let txn = self.inner.read_txn()?;
        Ok(meilisearch_types::settings::settings(
            &self.inner,
            &txn,
            meilisearch_types::settings::SecretPolicy::RevealSecrets,
        )?
        .into_unchecked())
    }
    pub fn update_settings(&self, settings: &Settings) -> Result<()> {
        let checked = settings.clone().validate()?.check();
        let mut txn = self.inner.write_txn()?;
        let config = milli::update::IndexerConfig::default();
        let mut builder = milli::update::Settings::new(&mut txn, &self.inner, &config);
        meilisearch_types::settings::apply_settings_to_builder(&checked, &mut builder);
        builder.execute(
            &milli::MustStopProcessing::default(),
            &milli::progress::Progress::quiet(),
            &self.ip_policy.clone(),
            Default::default(),
        )?;
        txn.commit()?;
        Ok(())
    }
    pub fn reset_settings(&self) -> Result<()> {
        self.update_settings(&meilisearch_types::settings::Settings::cleared().into_unchecked())
    }
    pub fn primary_key(&self) -> Result<Option<String>> {
        let txn = self.inner.read_txn()?;
        Ok(self.inner.primary_key(&txn)?.map(str::to_owned))
    }
    pub fn get_ranking_rules(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["rankingRules"].clone(),
        )?)
    }
    pub fn update_ranking_rules(&self, rules: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"rankingRules": rules}))?;
        self.update_settings(&settings)
    }
    pub fn reset_ranking_rules(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"rankingRules": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_distinct_attribute(&self) -> Result<Option<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["distinctAttribute"].clone(),
        )?)
    }
    pub fn update_distinct_attribute(&self, attr: &str) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"distinctAttribute": attr}))?;
        self.update_settings(&settings)
    }
    pub fn reset_distinct_attribute(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"distinctAttribute": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_searchable_attributes(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["searchableAttributes"].clone(),
        )?)
    }
    pub fn update_searchable_attributes(&self, attrs: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"searchableAttributes": attrs}))?;
        self.update_settings(&settings)
    }
    pub fn reset_searchable_attributes(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"searchableAttributes": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_displayed_attributes(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["displayedAttributes"].clone(),
        )?)
    }
    pub fn update_displayed_attributes(&self, attrs: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"displayedAttributes": attrs}))?;
        self.update_settings(&settings)
    }
    pub fn reset_displayed_attributes(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"displayedAttributes": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_synonyms(&self) -> Result<std::collections::HashMap<String, Vec<String>>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["synonyms"].clone(),
        )?)
    }
    pub fn update_synonyms(
        &self,
        synonyms: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"synonyms": synonyms}))?;
        self.update_settings(&settings)
    }
    pub fn reset_synonyms(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"synonyms": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_stop_words(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["stopWords"].clone(),
        )?)
    }
    pub fn update_stop_words(&self, words: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"stopWords": words}))?;
        self.update_settings(&settings)
    }
    pub fn reset_stop_words(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"stopWords": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_filterable_attributes(&self) -> Result<Vec<FilterableAttributesRule>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["filterableAttributes"].clone(),
        )?)
    }
    pub fn update_filterable_attributes(&self, attrs: &[FilterableAttributesRule]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"filterableAttributes": attrs}))?;
        self.update_settings(&settings)
    }
    pub fn reset_filterable_attributes(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"filterableAttributes": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_sortable_attributes(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["sortableAttributes"].clone(),
        )?)
    }
    pub fn update_sortable_attributes(&self, attrs: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"sortableAttributes": attrs}))?;
        self.update_settings(&settings)
    }
    pub fn reset_sortable_attributes(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"sortableAttributes": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_typo_tolerance(&self) -> Result<TypoTolerance> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["typoTolerance"].clone(),
        )?)
    }
    pub fn update_typo_tolerance(&self, config: &TypoTolerance) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"typoTolerance": config}))?;
        self.update_settings(&settings)
    }
    pub fn reset_typo_tolerance(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"typoTolerance": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_pagination(&self) -> Result<Pagination> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["pagination"].clone(),
        )?)
    }
    pub fn update_pagination(&self, config: &Pagination) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"pagination": config}))?;
        self.update_settings(&settings)
    }
    pub fn reset_pagination(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"pagination": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_faceting(&self) -> Result<Faceting> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["faceting"].clone(),
        )?)
    }
    pub fn update_faceting(&self, config: &Faceting) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"faceting": config}))?;
        self.update_settings(&settings)
    }
    pub fn reset_faceting(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"faceting": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_dictionary(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["dictionary"].clone(),
        )?)
    }
    pub fn update_dictionary(&self, words: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"dictionary": words}))?;
        self.update_settings(&settings)
    }
    pub fn reset_dictionary(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"dictionary": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_separator_tokens(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["separatorTokens"].clone(),
        )?)
    }
    pub fn update_separator_tokens(&self, tokens: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"separatorTokens": tokens}))?;
        self.update_settings(&settings)
    }
    pub fn reset_separator_tokens(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"separatorTokens": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_non_separator_tokens(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["nonSeparatorTokens"].clone(),
        )?)
    }
    pub fn update_non_separator_tokens(&self, tokens: &[String]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"nonSeparatorTokens": tokens}))?;
        self.update_settings(&settings)
    }
    pub fn reset_non_separator_tokens(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"nonSeparatorTokens": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_proximity_precision(&self) -> Result<ProximityPrecision> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["proximityPrecision"].clone(),
        )?)
    }
    pub fn update_proximity_precision(&self, precision: ProximityPrecision) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"proximityPrecision": precision}))?;
        self.update_settings(&settings)
    }
    pub fn reset_proximity_precision(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"proximityPrecision": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_facet_search(&self) -> Result<bool> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["facetSearch"].clone(),
        )?)
    }
    pub fn update_facet_search(&self, enabled: bool) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"facetSearch": enabled}))?;
        self.update_settings(&settings)
    }
    pub fn reset_facet_search(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"facetSearch": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_prefix_search(&self) -> Result<PrefixSearch> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["prefixSearch"].clone(),
        )?)
    }
    pub fn update_prefix_search(&self, mode: PrefixSearch) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"prefixSearch": mode}))?;
        self.update_settings(&settings)
    }
    pub fn reset_prefix_search(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"prefixSearch": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_search_cutoff_ms(&self) -> Result<Option<u64>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["searchCutoffMs"].clone(),
        )?)
    }
    pub fn update_search_cutoff_ms(&self, ms: u64) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"searchCutoffMs": ms}))?;
        self.update_settings(&settings)
    }
    pub fn reset_search_cutoff_ms(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"searchCutoffMs": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
    pub fn get_localized_attributes(&self) -> Result<Option<Vec<LocalizedAttribute>>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["localizedAttributes"].clone(),
        )?)
    }
    pub fn update_localized_attributes(&self, attrs: &[LocalizedAttribute]) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"localizedAttributes": attrs}))?;
        self.update_settings(&settings)
    }
    pub fn reset_localized_attributes(&self) -> Result<()> {
        let settings = serde_json::from_value(
            serde_json::json!({"localizedAttributes": serde_json::Value::Null}),
        )?;
        self.update_settings(&settings)
    }
    pub fn get_embedders(
        &self,
    ) -> Result<Option<std::collections::HashMap<String, EmbedderConfig>>> {
        Ok(serde_json::from_value(
            serde_json::to_value(self.get_settings()?)?["embedders"].clone(),
        )?)
    }
    pub fn update_embedders(
        &self,
        embedders: &std::collections::HashMap<String, EmbedderConfig>,
    ) -> Result<()> {
        let settings = serde_json::from_value(serde_json::json!({"embedders": embedders}))?;
        self.update_settings(&settings)
    }
    pub fn reset_embedders(&self) -> Result<()> {
        let settings =
            serde_json::from_value(serde_json::json!({"embedders": serde_json::Value::Null}))?;
        self.update_settings(&settings)
    }
}
