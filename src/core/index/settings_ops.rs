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

/// Generate get/update/reset methods for a single `Settings` field.
///
/// Each invocation produces three public methods on `Index`:
/// - `get_{name}(&self) -> Result<Option<T>>`
/// - `update_{name}(&self, val: T) -> Result<()>`
/// - `reset_{name}(&self) -> Result<()>`
macro_rules! settings_accessor {
    ($name:ident, $ty:ty, $reset:expr) => {
        paste::paste! {
            #[doc = "Get the " $name " setting."]
            pub fn [<get_ $name>](&self) -> Result<Option<$ty>> {
                let settings = self.get_settings()?;
                Ok(settings.$name)
            }

            #[doc = "Update the " $name " setting."]
            pub fn [<update_ $name>](&self, val: $ty) -> Result<()> {
                let settings = Settings::new().[<with_ $name>](val);
                self.update_settings(&settings)
            }

            #[doc = "Reset the " $name " to its default value."]
            pub fn [<reset_ $name>](&self) -> Result<()> {
                self.execute_settings_reset($reset)
            }
        }
    };
}

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

    settings_accessor!(displayed_attributes,   Vec<String>,                        |s| s.reset_displayed_fields());
    settings_accessor!(searchable_attributes,  Vec<String>,                        |s| s.reset_searchable_fields());
    settings_accessor!(filterable_attributes,  Vec<String>,                        |s| s.reset_filterable_fields());
    settings_accessor!(sortable_attributes,    BTreeSet<String>,                   |s| s.reset_sortable_fields());
    settings_accessor!(ranking_rules,          Vec<String>,                        |s| s.reset_criteria());
    settings_accessor!(stop_words,             BTreeSet<String>,                   |s| s.reset_stop_words());
    settings_accessor!(non_separator_tokens,   BTreeSet<String>,                   |s| s.reset_non_separator_tokens());
    settings_accessor!(separator_tokens,       BTreeSet<String>,                   |s| s.reset_separator_tokens());
    settings_accessor!(dictionary,             BTreeSet<String>,                   |s| s.reset_dictionary());
    settings_accessor!(synonyms,              BTreeMap<String, Vec<String>>,       |s| s.reset_synonyms());
    settings_accessor!(distinct_attribute,     String,                             |s| s.reset_distinct_field());
    settings_accessor!(proximity_precision,    ProximityPrecision,                 |s| s.reset_proximity_precision());
    settings_accessor!(typo_tolerance,         TypoToleranceSettings,              |s| {
        s.reset_authorize_typos();
        s.reset_min_word_len_one_typo();
        s.reset_min_word_len_two_typos();
        s.reset_exact_words();
        s.reset_exact_attributes();
    });
    settings_accessor!(faceting,               FacetingSettings,                   |s| s.reset_max_values_per_facet());
    settings_accessor!(pagination,             PaginationSettings,                 |s| s.reset_pagination_max_total_hits());
    settings_accessor!(embedders,              HashMap<String, EmbedderSettings>,  |s| s.reset_embedder_settings());
    settings_accessor!(search_cutoff_ms,       u64,                                |s| s.reset_search_cutoff());
    settings_accessor!(localized_attributes,   Vec<LocalizedAttributeRule>,        |s| s.reset_localized_attributes_rules());
    settings_accessor!(facet_search,           bool,                               |s| s.reset_facet_search());
    settings_accessor!(prefix_search,          String,                             |s| s.reset_prefix_search());

    /// Get the primary key field name for this index.
    ///
    /// Returns `None` if no primary key has been set yet (the index has no documents).
    pub fn primary_key(&self) -> Result<Option<String>> {
        let rtxn = self.inner.read_txn().map_err(Error::Heed)?;
        let pk = self.inner.primary_key(&rtxn).map_err(Error::Heed)?;
        Ok(pk.map(|s| s.to_string()))
    }
}
