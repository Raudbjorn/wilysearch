use milli::progress::EmbedderStats;
use milli::update::IndexerConfig;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::core::error::{Error, Result};
use crate::core::settings::{
    read_settings_from_index, EmbedderSettings, FacetingSettings, LocalizedAttributeRule,
    PaginationSettings, ProximityPrecision, Settings, SettingsApplier, TypoToleranceSettings,
};

use super::Index;

impl Index {
    // ========================================================================
    // Settings Operations
    // ========================================================================

    /// Get the current settings of the index.
    ///
    /// Returns a [`Settings`] struct containing all configured index settings
    /// including searchable/filterable/sortable attributes, ranking rules,
    /// embedders, and more.
    pub fn get_settings(&self) -> Result<Settings> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        read_settings_from_index(&rtxn, &self.inner)
    }

    /// Update the index settings.
    ///
    /// Only the fields that are set (not `None`) in the provided [`Settings`]
    /// will be updated. Fields set to `None` are left unchanged.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use wilysearch::core::{Meilisearch, MeilisearchOptions};
    /// # let meili = Meilisearch::new(MeilisearchOptions::default()).unwrap();
    /// # let index = meili.create_index("movies", Some("id")).unwrap();
    /// use wilysearch::core::{Settings, EmbedderSettings};
    ///
    /// let settings = Settings::new()
    ///     .with_searchable_attributes(vec!["title".to_string(), "content".to_string()])
    ///     .with_filterable_attributes(vec!["category".to_string(), "price".to_string()])
    ///     .with_embedder("default", EmbedderSettings::openai("your-api-key"));
    ///
    /// index.update_settings(&settings)?;
    /// # Ok::<(), wilysearch::core::Error>(())
    /// ```
    pub fn update_settings(&self, settings: &Settings) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();

        let milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);

        let applier = SettingsApplier { builder: milli_settings };
        let milli_settings = applier.apply(settings)?;

        // Execute the settings update
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::default();

        milli_settings
            .execute(&|| false, &progress, &ip_policy, embedder_stats)
            .map_err(Error::Milli)?;

        wtxn.commit().map_err(Error::Heed)?;

        Ok(())
    }

    /// Reset all settings to their default values.
    ///
    /// This resets:
    /// - Searchable attributes (all fields become searchable)
    /// - Displayed attributes (all fields become displayed)
    /// - Filterable attributes (cleared)
    /// - Sortable attributes (cleared)
    /// - Ranking rules (reset to default)
    /// - Stop words (cleared)
    /// - Synonyms (cleared)
    /// - Embedders (cleared)
    /// - Distinct attribute (cleared)
    /// - Typo tolerance (reset to default)
    ///
    /// Note: This does NOT delete documents.
    pub fn reset_settings(&self) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();

        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);

        // Reset all settings to their defaults
        milli_settings.reset_searchable_fields();
        milli_settings.reset_displayed_fields();
        milli_settings.reset_filterable_fields();
        milli_settings.reset_sortable_fields();
        milli_settings.reset_criteria();
        milli_settings.reset_stop_words();
        milli_settings.reset_non_separator_tokens();
        milli_settings.reset_separator_tokens();
        milli_settings.reset_dictionary();
        milli_settings.reset_synonyms();
        milli_settings.reset_embedder_settings();
        milli_settings.reset_distinct_field();
        milli_settings.reset_proximity_precision();
        milli_settings.reset_authorize_typos();
        milli_settings.reset_min_word_len_one_typo();
        milli_settings.reset_min_word_len_two_typos();
        milli_settings.reset_exact_words();
        milli_settings.reset_exact_attributes();
        milli_settings.reset_disable_on_numbers();
        milli_settings.reset_max_values_per_facet();
        milli_settings.reset_pagination_max_total_hits();
        milli_settings.reset_search_cutoff();
        milli_settings.reset_localized_attributes_rules();
        milli_settings.reset_facet_search();
        milli_settings.reset_prefix_search();

        // Execute the settings update
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        let progress = milli::progress::Progress::default();

        milli_settings
            .execute(&|| false, &progress, &ip_policy, embedder_stats)
            .map_err(Error::Milli)?;

        wtxn.commit().map_err(Error::Heed)?;

        Ok(())
    }

    /// Helper: open a write transaction, create a milli settings builder,
    /// apply a single reset closure, execute, and commit.
    fn execute_settings_reset(
        &self,
        apply: impl FnOnce(&mut milli::update::Settings<'_, '_, '_>),
    ) -> Result<()> {
        let mut wtxn = self.inner.write_txn().map_err(Error::Heed)?;
        let indexer_config = IndexerConfig::default();
        let mut milli_settings =
            milli::update::Settings::new(&mut wtxn, &self.inner, &indexer_config);
        apply(&mut milli_settings);
        let ip_policy = http_client::policy::IpPolicy::deny_all_local_ips();
        let embedder_stats = Arc::new(EmbedderStats::default());
        milli_settings
            .execute(
                &|| false,
                &milli::progress::Progress::default(),
                &ip_policy,
                embedder_stats,
            )
            .map_err(Error::Milli)?;
        wtxn.commit().map_err(Error::Heed)?;
        Ok(())
    }

    // ========================================================================
    // Individual Settings Accessors
    // ========================================================================

    // --- displayed_attributes ---

    /// Get the displayed attributes setting.
    pub fn get_displayed_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.displayed_attributes)
    }

    /// Update the displayed attributes setting.
    pub fn update_displayed_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_displayed_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the displayed attributes to their default value.
    pub fn reset_displayed_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_displayed_fields())
    }

    // --- searchable_attributes ---

    /// Get the searchable attributes setting.
    pub fn get_searchable_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.searchable_attributes)
    }

    /// Update the searchable attributes setting.
    pub fn update_searchable_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_searchable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the searchable attributes to their default value.
    pub fn reset_searchable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_searchable_fields())
    }

    // --- filterable_attributes ---

    /// Get the filterable attributes setting.
    pub fn get_filterable_attributes(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.filterable_attributes)
    }

    /// Update the filterable attributes setting.
    pub fn update_filterable_attributes(&self, attrs: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_filterable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the filterable attributes to their default value.
    pub fn reset_filterable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_filterable_fields())
    }

    // --- sortable_attributes ---

    /// Get the sortable attributes setting.
    pub fn get_sortable_attributes(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.sortable_attributes)
    }

    /// Update the sortable attributes setting.
    pub fn update_sortable_attributes(&self, attrs: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_sortable_attributes(attrs);
        self.update_settings(&settings)
    }

    /// Reset the sortable attributes to their default value.
    pub fn reset_sortable_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_sortable_fields())
    }

    // --- ranking_rules ---

    /// Get the ranking rules setting.
    pub fn get_ranking_rules(&self) -> Result<Option<Vec<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.ranking_rules)
    }

    /// Update the ranking rules setting.
    pub fn update_ranking_rules(&self, rules: Vec<String>) -> Result<()> {
        let settings = Settings::new().with_ranking_rules(rules);
        self.update_settings(&settings)
    }

    /// Reset the ranking rules to their default value.
    pub fn reset_ranking_rules(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_criteria())
    }

    // --- stop_words ---

    /// Get the stop words setting.
    pub fn get_stop_words(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.stop_words)
    }

    /// Update the stop words setting.
    pub fn update_stop_words(&self, words: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_stop_words(words);
        self.update_settings(&settings)
    }

    /// Reset the stop words to their default value.
    pub fn reset_stop_words(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_stop_words())
    }

    // --- non_separator_tokens ---

    /// Get the non-separator tokens setting.
    pub fn get_non_separator_tokens(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.non_separator_tokens)
    }

    /// Update the non-separator tokens setting.
    pub fn update_non_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_non_separator_tokens(tokens);
        self.update_settings(&settings)
    }

    /// Reset the non-separator tokens to their default value.
    pub fn reset_non_separator_tokens(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_non_separator_tokens())
    }

    // --- separator_tokens ---

    /// Get the separator tokens setting.
    pub fn get_separator_tokens(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.separator_tokens)
    }

    /// Update the separator tokens setting.
    pub fn update_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_separator_tokens(tokens);
        self.update_settings(&settings)
    }

    /// Reset the separator tokens to their default value.
    pub fn reset_separator_tokens(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_separator_tokens())
    }

    // --- dictionary ---

    /// Get the dictionary setting.
    pub fn get_dictionary(&self) -> Result<Option<BTreeSet<String>>> {
        let settings = self.get_settings()?;
        Ok(settings.dictionary)
    }

    /// Update the dictionary setting.
    pub fn update_dictionary(&self, words: BTreeSet<String>) -> Result<()> {
        let settings = Settings::new().with_dictionary(words);
        self.update_settings(&settings)
    }

    /// Reset the dictionary to its default value.
    pub fn reset_dictionary(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_dictionary())
    }

    // --- synonyms ---

    /// Get the synonyms setting.
    pub fn get_synonyms(&self) -> Result<Option<BTreeMap<String, Vec<String>>>> {
        let settings = self.get_settings()?;
        Ok(settings.synonyms)
    }

    /// Update the synonyms setting.
    pub fn update_synonyms(&self, synonyms: BTreeMap<String, Vec<String>>) -> Result<()> {
        let settings = Settings::new().with_synonyms(synonyms);
        self.update_settings(&settings)
    }

    /// Reset the synonyms to their default value.
    pub fn reset_synonyms(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_synonyms())
    }

    // --- distinct_attribute ---

    /// Get the distinct attribute setting.
    pub fn get_distinct_attribute(&self) -> Result<Option<String>> {
        let settings = self.get_settings()?;
        Ok(settings.distinct_attribute)
    }

    /// Update the distinct attribute setting.
    pub fn update_distinct_attribute(&self, attr: String) -> Result<()> {
        let settings = Settings::new().with_distinct_attribute(attr);
        self.update_settings(&settings)
    }

    /// Reset the distinct attribute to its default value.
    pub fn reset_distinct_attribute(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_distinct_field())
    }

    // --- proximity_precision ---

    /// Get the proximity precision setting.
    pub fn get_proximity_precision(&self) -> Result<Option<ProximityPrecision>> {
        let settings = self.get_settings()?;
        Ok(settings.proximity_precision)
    }

    /// Update the proximity precision setting.
    pub fn update_proximity_precision(&self, precision: ProximityPrecision) -> Result<()> {
        let settings = Settings::new().with_proximity_precision(precision);
        self.update_settings(&settings)
    }

    /// Reset the proximity precision to its default value.
    pub fn reset_proximity_precision(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_proximity_precision())
    }

    // --- typo_tolerance ---

    /// Get the typo tolerance setting.
    pub fn get_typo_tolerance(&self) -> Result<Option<TypoToleranceSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.typo_tolerance)
    }

    /// Update the typo tolerance setting.
    pub fn update_typo_tolerance(&self, typo_tolerance: TypoToleranceSettings) -> Result<()> {
        let settings = Settings::new().with_typo_tolerance(typo_tolerance);
        self.update_settings(&settings)
    }

    /// Reset the typo tolerance to its default value.
    pub fn reset_typo_tolerance(&self) -> Result<()> {
        self.execute_settings_reset(|s| {
            s.reset_authorize_typos();
            s.reset_min_word_len_one_typo();
            s.reset_min_word_len_two_typos();
            s.reset_exact_words();
            s.reset_exact_attributes();
        })
    }

    // --- faceting ---

    /// Get the faceting setting.
    pub fn get_faceting(&self) -> Result<Option<FacetingSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.faceting)
    }

    /// Update the faceting setting.
    pub fn update_faceting(&self, faceting: FacetingSettings) -> Result<()> {
        let settings = Settings::new().with_faceting(faceting);
        self.update_settings(&settings)
    }

    /// Reset the faceting to its default value.
    pub fn reset_faceting(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_max_values_per_facet())
    }

    // --- pagination ---

    /// Get the pagination setting.
    pub fn get_pagination(&self) -> Result<Option<PaginationSettings>> {
        let settings = self.get_settings()?;
        Ok(settings.pagination)
    }

    /// Update the pagination setting.
    pub fn update_pagination(&self, pagination: PaginationSettings) -> Result<()> {
        let settings = Settings::new().with_pagination(pagination);
        self.update_settings(&settings)
    }

    /// Reset the pagination to its default value.
    pub fn reset_pagination(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_pagination_max_total_hits())
    }

    // --- embedders ---

    /// Get the embedders setting.
    pub fn get_embedders(&self) -> Result<Option<HashMap<String, EmbedderSettings>>> {
        let settings = self.get_settings()?;
        Ok(settings.embedders)
    }

    /// Update the embedders setting.
    pub fn update_embedders(&self, embedders: HashMap<String, EmbedderSettings>) -> Result<()> {
        let settings = Settings::new().with_embedders(embedders);
        self.update_settings(&settings)
    }

    /// Reset the embedders to their default value.
    pub fn reset_embedders(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_embedder_settings())
    }

    // --- search_cutoff_ms ---

    /// Get the search cutoff in milliseconds setting.
    pub fn get_search_cutoff_ms(&self) -> Result<Option<u64>> {
        let settings = self.get_settings()?;
        Ok(settings.search_cutoff_ms)
    }

    /// Update the search cutoff in milliseconds setting.
    pub fn update_search_cutoff_ms(&self, ms: u64) -> Result<()> {
        let settings = Settings::new().with_search_cutoff_ms(ms);
        self.update_settings(&settings)
    }

    /// Reset the search cutoff to its default value.
    pub fn reset_search_cutoff_ms(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_search_cutoff())
    }

    // --- localized_attributes ---

    /// Get the localized attributes setting.
    pub fn get_localized_attributes(&self) -> Result<Option<Vec<LocalizedAttributeRule>>> {
        let settings = self.get_settings()?;
        Ok(settings.localized_attributes)
    }

    /// Update the localized attributes setting.
    pub fn update_localized_attributes(&self, rules: Vec<LocalizedAttributeRule>) -> Result<()> {
        let settings = Settings::new().with_localized_attributes(rules);
        self.update_settings(&settings)
    }

    /// Reset the localized attributes to their default value.
    pub fn reset_localized_attributes(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_localized_attributes_rules())
    }

    // --- facet_search ---

    /// Get the facet search setting.
    pub fn get_facet_search(&self) -> Result<Option<bool>> {
        let settings = self.get_settings()?;
        Ok(settings.facet_search)
    }

    /// Update the facet search setting.
    pub fn update_facet_search(&self, enabled: bool) -> Result<()> {
        let settings = Settings::new().with_facet_search(enabled);
        self.update_settings(&settings)
    }

    /// Reset the facet search to its default value.
    pub fn reset_facet_search(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_facet_search())
    }

    // --- prefix_search ---

    /// Get the prefix search setting.
    pub fn get_prefix_search(&self) -> Result<Option<String>> {
        let settings = self.get_settings()?;
        Ok(settings.prefix_search)
    }

    /// Update the prefix search setting.
    pub fn update_prefix_search(&self, mode: String) -> Result<()> {
        let settings = Settings::new().with_prefix_search(mode);
        self.update_settings(&settings)
    }

    /// Reset the prefix search to its default value.
    pub fn reset_prefix_search(&self) -> Result<()> {
        self.execute_settings_reset(|s| s.reset_prefix_search())
    }

    /// Get the primary key field name for this index.
    ///
    /// Returns `None` if no primary key has been set yet (the index has no documents).
    pub fn primary_key(&self) -> Result<Option<String>> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        let pk = self.inner.primary_key(&rtxn).map_err(Error::Heed)?;
        Ok(pk.map(|s| s.to_string()))
    }
}
