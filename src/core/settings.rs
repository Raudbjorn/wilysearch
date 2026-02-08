//! Settings module for meilisearch-lib
//!
//! Provides a simplified interface for configuring Meilisearch index settings,
//! including searchable/filterable/sortable attributes, ranking rules, embedders, etc.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::Error;
use milli::update::Setting;
use milli::vector::settings::EmbeddingSettings as MilliEmbeddingSettings;
use milli::FilterableAttributesRule;
use serde::{Deserialize, Serialize};

/// Embedder source types supported by Meilisearch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EmbedderSource {
    /// OpenAI embeddings API
    #[default]
    OpenAi,
    /// HuggingFace models (local inference)
    HuggingFace,
    /// Ollama local models
    Ollama,
    /// User-provided embeddings (manual)
    UserProvided,
    /// Generic REST API endpoint
    Rest,
    /// Composite (multi-source) embedder.
    Composite,
}

impl From<EmbedderSource> for milli::vector::settings::EmbedderSource {
    fn from(source: EmbedderSource) -> Self {
        match source {
            EmbedderSource::OpenAi => milli::vector::settings::EmbedderSource::OpenAi,
            EmbedderSource::HuggingFace => milli::vector::settings::EmbedderSource::HuggingFace,
            EmbedderSource::Ollama => milli::vector::settings::EmbedderSource::Ollama,
            EmbedderSource::UserProvided => milli::vector::settings::EmbedderSource::UserProvided,
            EmbedderSource::Rest => milli::vector::settings::EmbedderSource::Rest,
            EmbedderSource::Composite => milli::vector::settings::EmbedderSource::Composite,
        }
    }
}

impl From<milli::vector::settings::EmbedderSource> for EmbedderSource {
    fn from(source: milli::vector::settings::EmbedderSource) -> Self {
        match source {
            milli::vector::settings::EmbedderSource::OpenAi => EmbedderSource::OpenAi,
            milli::vector::settings::EmbedderSource::HuggingFace => EmbedderSource::HuggingFace,
            milli::vector::settings::EmbedderSource::Ollama => EmbedderSource::Ollama,
            milli::vector::settings::EmbedderSource::UserProvided => EmbedderSource::UserProvided,
            milli::vector::settings::EmbedderSource::Rest => EmbedderSource::Rest,
            milli::vector::settings::EmbedderSource::Composite => EmbedderSource::Composite,
        }
    }
}

/// Configuration for an embedder
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedderSettings {
    /// The source/type of embedder to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<EmbedderSource>,

    /// Model name (e.g., "text-embedding-3-small" for OpenAI)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// API key for remote embedders (OpenAI, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// URL for Ollama or REST embedders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Expected embedding dimensions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,

    /// Liquid template for rendering documents to text before embedding
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_template: Option<String>,

    /// Maximum bytes for the rendered document template
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_template_max_bytes: Option<usize>,

    /// Whether to use binary quantization for embeddings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_quantized: Option<bool>,

    /// Model revision (for HuggingFace models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,

    /// Additional headers for REST embedders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Request template for REST embedders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<serde_json::Value>,

    /// Response template for REST embedders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,
}

impl EmbedderSettings {
    /// Create a new OpenAI embedder configuration
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            source: Some(EmbedderSource::OpenAi),
            api_key: Some(api_key.into()),
            ..Default::default()
        }
    }

    /// Create a new OpenAI embedder with a specific model
    pub fn openai_with_model(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            source: Some(EmbedderSource::OpenAi),
            api_key: Some(api_key.into()),
            model: Some(model.into()),
            ..Default::default()
        }
    }

    /// Create a new Ollama embedder configuration
    pub fn ollama(url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            source: Some(EmbedderSource::Ollama),
            url: Some(url.into()),
            model: Some(model.into()),
            ..Default::default()
        }
    }

    /// Create a new HuggingFace embedder configuration
    pub fn huggingface(model: impl Into<String>) -> Self {
        Self {
            source: Some(EmbedderSource::HuggingFace),
            model: Some(model.into()),
            ..Default::default()
        }
    }

    /// Create a new user-provided embedder configuration
    pub fn user_provided(dimensions: usize) -> Self {
        Self {
            source: Some(EmbedderSource::UserProvided),
            dimensions: Some(dimensions),
            ..Default::default()
        }
    }

    /// Create a new REST embedder configuration
    pub fn rest(url: impl Into<String>) -> Self {
        Self {
            source: Some(EmbedderSource::Rest),
            url: Some(url.into()),
            ..Default::default()
        }
    }

    /// Set the document template
    pub fn with_document_template(mut self, template: impl Into<String>) -> Self {
        self.document_template = Some(template.into());
        self
    }

    /// Set the dimensions
    pub fn with_dimensions(mut self, dimensions: usize) -> Self {
        self.dimensions = Some(dimensions);
        self
    }
}

impl From<EmbedderSettings> for Setting<MilliEmbeddingSettings> {
    fn from(settings: EmbedderSettings) -> Self {
        let mut milli_settings = MilliEmbeddingSettings::default();

        if let Some(source) = settings.source {
            milli_settings.source = Setting::Set(source.into());
        }
        if let Some(model) = settings.model {
            milli_settings.model = Setting::Set(model);
        }
        if let Some(api_key) = settings.api_key {
            milli_settings.api_key = Setting::Set(api_key);
        }
        if let Some(url) = settings.url {
            milli_settings.url = Setting::Set(url);
        }
        if let Some(dimensions) = settings.dimensions {
            milli_settings.dimensions = Setting::Set(dimensions);
        }
        if let Some(document_template) = settings.document_template {
            milli_settings.document_template = Setting::Set(document_template);
        }
        if let Some(document_template_max_bytes) = settings.document_template_max_bytes {
            milli_settings.document_template_max_bytes = Setting::Set(document_template_max_bytes);
        }
        if let Some(binary_quantized) = settings.binary_quantized {
            milli_settings.binary_quantized = Setting::Set(binary_quantized);
        }
        if let Some(revision) = settings.revision {
            milli_settings.revision = Setting::Set(revision);
        }
        if let Some(headers) = settings.headers {
            milli_settings.headers = Setting::Set(headers.into_iter().collect());
        }
        if let Some(request) = settings.request {
            milli_settings.request = Setting::Set(request);
        }
        if let Some(response) = settings.response {
            milli_settings.response = Setting::Set(response);
        }

        Setting::Set(milli_settings)
    }
}

impl From<MilliEmbeddingSettings> for EmbedderSettings {
    fn from(milli_settings: MilliEmbeddingSettings) -> Self {
        Self {
            source: milli_settings.source.set().map(|s| s.into()),
            model: milli_settings.model.set(),
            api_key: milli_settings.api_key.set(),
            url: milli_settings.url.set(),
            dimensions: milli_settings.dimensions.set(),
            document_template: milli_settings.document_template.set(),
            document_template_max_bytes: milli_settings.document_template_max_bytes.set(),
            binary_quantized: milli_settings.binary_quantized.set(),
            revision: milli_settings.revision.set(),
            headers: milli_settings.headers.set().map(|h| h.into_iter().collect()),
            request: milli_settings.request.set(),
            response: milli_settings.response.set(),
        }
    }
}

/// Typo tolerance configuration for an index.
///
/// Controls whether typo correction is enabled, how many characters a word
/// must have before one or two typos are tolerated, and per-word/attribute
/// exclusions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypoToleranceSettings {
    /// Whether typo tolerance is enabled. Default: `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Minimum word lengths before 1 or 2 typos are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_word_size_for_typos: Option<MinWordSizeForTypos>,
    /// Words that must match exactly (no typos tolerated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_words: Option<BTreeSet<String>>,
    /// Attributes where typo tolerance is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_attributes: Option<BTreeSet<String>>,
    /// Whether typo tolerance is disabled on numeric tokens. Default: `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_numbers: Option<bool>,
}

/// Minimum word length thresholds for tolerating typos.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinWordSizeForTypos {
    /// Minimum characters for a word to tolerate one typo. Default: 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_typo: Option<u8>,
    /// Minimum characters for a word to tolerate two typos. Default: 9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_typos: Option<u8>,
}

/// Faceting configuration for an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetingSettings {
    /// Maximum number of facet values returned per attribute. Default: 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values_per_facet: Option<usize>,
    /// Sort order for facet values, keyed by attribute name (or `"*"` for all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_facet_values_by: Option<BTreeMap<String, FacetValuesSort>>,
}

/// Sort order for facet values in search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FacetValuesSort {
    /// Sort facet values alphabetically.
    Alpha,
    /// Sort facet values by document count (descending).
    Count,
}

/// Pagination configuration for an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationSettings {
    /// Maximum total number of hits that can be browsed. Default: 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_hits: Option<usize>,
}

/// Precision level for the proximity ranking rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProximityPrecision {
    /// Compute proximity at word granularity (most precise, higher indexing cost).
    ByWord,
    /// Compute proximity at attribute granularity (faster indexing).
    ByAttribute,
}

/// Rule associating locales with attribute name patterns for language-specific tokenization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedAttributeRule {
    /// Glob patterns matching attribute names (e.g. `["title_*", "description_*"]`).
    pub attribute_patterns: Vec<String>,
    /// Language codes to apply to matched attributes (e.g. `["eng", "fra"]`).
    pub locales: Vec<String>,
}

/// Index settings configuration matching the Meilisearch HTTP API shape.
///
/// Use the builder-style `with_*` methods to construct a `Settings` value for
/// [`Index::update_settings`](crate::Index::update_settings). Fields left as
/// `None` are not changed when the settings are applied.
///
/// # Example
///
/// ```no_run
/// use wilysearch::core::{Settings, EmbedderSettings};
///
/// let settings = Settings::new()
///     .with_searchable_attributes(vec!["title".into(), "overview".into()])
///     .with_filterable_attributes(vec!["genre".into(), "year".into()])
///     .with_embedder("default", EmbedderSettings::openai("sk-..."));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Attributes whose values are shown in returned documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed_attributes: Option<Vec<String>>,
    /// Attributes searched by keyword queries (order defines weight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searchable_attributes: Option<Vec<String>>,
    /// Attributes that can be used in filter expressions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filterable_attributes: Option<Vec<String>>,
    /// Attributes that can be used in sort expressions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sortable_attributes: Option<BTreeSet<String>>,
    /// Ordered list of ranking rules (e.g. `["words", "typo", "proximity"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_rules: Option<Vec<String>>,
    /// Words ignored during search (e.g. `["the", "a", "an"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_words: Option<BTreeSet<String>>,
    /// Tokens that should not be treated as word separators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_separator_tokens: Option<BTreeSet<String>>,
    /// Additional tokens that should be treated as word separators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator_tokens: Option<BTreeSet<String>>,
    /// Custom dictionary words that the tokenizer should treat as single tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionary: Option<BTreeSet<String>>,
    /// Synonym mappings (word -> list of synonyms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synonyms: Option<BTreeMap<String, Vec<String>>>,
    /// Attribute used for de-duplicating search results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct_attribute: Option<String>,
    /// Precision level for the proximity ranking rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proximity_precision: Option<ProximityPrecision>,
    /// Typo tolerance configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typo_tolerance: Option<TypoToleranceSettings>,
    /// Faceting configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faceting: Option<FacetingSettings>,
    /// Pagination limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationSettings>,
    /// Named embedder configurations for vector/hybrid search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedders: Option<HashMap<String, EmbedderSettings>>,
    /// Maximum time in milliseconds before a search is aborted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_cutoff_ms: Option<u64>,
    /// Rules associating locales with attribute patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub localized_attributes: Option<Vec<LocalizedAttributeRule>>,
    /// Whether facet search is enabled. Default: `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_search: Option<bool>,
    /// Prefix search mode (`"indexingTime"` or `"disabled"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_search: Option<String>,
}

impl Settings {
    /// Create a new empty settings instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Set displayed attributes
    pub fn with_displayed_attributes(mut self, attrs: Vec<String>) -> Self {
        self.displayed_attributes = Some(attrs);
        self
    }

    /// Set searchable attributes
    pub fn with_searchable_attributes(mut self, attrs: Vec<String>) -> Self {
        self.searchable_attributes = Some(attrs);
        self
    }

    /// Set filterable attributes
    pub fn with_filterable_attributes(mut self, attrs: Vec<String>) -> Self {
        self.filterable_attributes = Some(attrs);
        self
    }

    /// Set sortable attributes
    pub fn with_sortable_attributes(mut self, attrs: BTreeSet<String>) -> Self {
        self.sortable_attributes = Some(attrs);
        self
    }

    /// Set ranking rules
    pub fn with_ranking_rules(mut self, rules: Vec<String>) -> Self {
        self.ranking_rules = Some(rules);
        self
    }

    /// Set stop words
    pub fn with_stop_words(mut self, words: BTreeSet<String>) -> Self {
        self.stop_words = Some(words);
        self
    }

    /// Set non-separator tokens
    pub fn with_non_separator_tokens(mut self, tokens: BTreeSet<String>) -> Self {
        self.non_separator_tokens = Some(tokens);
        self
    }

    /// Set separator tokens
    pub fn with_separator_tokens(mut self, tokens: BTreeSet<String>) -> Self {
        self.separator_tokens = Some(tokens);
        self
    }

    /// Set dictionary words
    pub fn with_dictionary(mut self, words: BTreeSet<String>) -> Self {
        self.dictionary = Some(words);
        self
    }

    /// Set synonyms
    pub fn with_synonyms(mut self, synonyms: BTreeMap<String, Vec<String>>) -> Self {
        self.synonyms = Some(synonyms);
        self
    }

    /// Set distinct attribute
    pub fn with_distinct_attribute(mut self, attr: impl Into<String>) -> Self {
        self.distinct_attribute = Some(attr.into());
        self
    }

    /// Set proximity precision
    pub fn with_proximity_precision(mut self, precision: ProximityPrecision) -> Self {
        self.proximity_precision = Some(precision);
        self
    }

    /// Set typo tolerance configuration
    pub fn with_typo_tolerance(mut self, settings: TypoToleranceSettings) -> Self {
        self.typo_tolerance = Some(settings);
        self
    }

    /// Set faceting configuration
    pub fn with_faceting(mut self, settings: FacetingSettings) -> Self {
        self.faceting = Some(settings);
        self
    }

    /// Set pagination configuration
    pub fn with_pagination(mut self, settings: PaginationSettings) -> Self {
        self.pagination = Some(settings);
        self
    }

    /// Set embedders configuration
    pub fn with_embedders(mut self, embedders: HashMap<String, EmbedderSettings>) -> Self {
        self.embedders = Some(embedders);
        self
    }

    /// Add a single embedder
    pub fn with_embedder(mut self, name: impl Into<String>, embedder: EmbedderSettings) -> Self {
        let embedders = self.embedders.get_or_insert_with(HashMap::new);
        embedders.insert(name.into(), embedder);
        self
    }

    /// Set search cutoff in milliseconds
    pub fn with_search_cutoff_ms(mut self, ms: u64) -> Self {
        self.search_cutoff_ms = Some(ms);
        self
    }

    /// Set localized attribute rules
    pub fn with_localized_attributes(mut self, rules: Vec<LocalizedAttributeRule>) -> Self {
        self.localized_attributes = Some(rules);
        self
    }

    /// Set facet search enabled/disabled
    pub fn with_facet_search(mut self, enabled: bool) -> Self {
        self.facet_search = Some(enabled);
        self
    }

    /// Set prefix search mode (e.g., "indexingTime" or "disabled")
    pub fn with_prefix_search(mut self, mode: impl Into<String>) -> Self {
        self.prefix_search = Some(mode.into());
        self
    }
}

/// Internal helper to convert Settings to milli Settings builder calls
pub(crate) struct SettingsApplier<'a, 't, 'i> {
    pub(crate) builder: milli::update::Settings<'a, 't, 'i>,
}

impl<'a, 't, 'i> SettingsApplier<'a, 't, 'i> {
    /// Apply our Settings to the milli Settings builder
    pub fn apply(mut self, settings: &Settings) -> super::Result<milli::update::Settings<'a, 't, 'i>> {
        if let Some(ref attrs) = settings.displayed_attributes {
            self.builder.set_displayed_fields(attrs.clone());
        }

        if let Some(ref attrs) = settings.searchable_attributes {
            self.builder.set_searchable_fields(attrs.clone());
        }

        if let Some(ref attrs) = settings.filterable_attributes {
            let rules: Vec<FilterableAttributesRule> =
                attrs.iter().map(|s| FilterableAttributesRule::Field(s.clone())).collect();
            self.builder.set_filterable_fields(rules);
        }

        if let Some(ref attrs) = settings.sortable_attributes {
            // milli expects HashSet<String>, convert from BTreeSet
            let hash_set: HashSet<String> = attrs.iter().cloned().collect();
            self.builder.set_sortable_fields(hash_set);
        }

        if let Some(ref rules) = settings.ranking_rules {
            let mut criteria: Vec<milli::Criterion> = Vec::with_capacity(rules.len());
            for r in rules {
                match r.parse() {
                    Ok(c) => criteria.push(c),
                    Err(e) => {
                        return Err(Error::Internal(format!(
                            "Invalid ranking rule '{r}': {e}"
                        )));
                    }
                }
            }
            self.builder.set_criteria(criteria);
        }

        if let Some(ref words) = settings.stop_words {
            self.builder.set_stop_words(words.clone());
        }

        if let Some(ref tokens) = settings.non_separator_tokens {
            self.builder.set_non_separator_tokens(tokens.clone());
        }

        if let Some(ref tokens) = settings.separator_tokens {
            self.builder.set_separator_tokens(tokens.clone());
        }

        if let Some(ref words) = settings.dictionary {
            self.builder.set_dictionary(words.clone());
        }

        if let Some(ref synonyms) = settings.synonyms {
            self.builder.set_synonyms(synonyms.clone());
        }

        if let Some(ref attr) = settings.distinct_attribute {
            self.builder.set_distinct_field(attr.clone());
        }

        if let Some(ref precision) = settings.proximity_precision {
            let milli_precision = match precision {
                ProximityPrecision::ByWord => milli::proximity::ProximityPrecision::ByWord,
                ProximityPrecision::ByAttribute => milli::proximity::ProximityPrecision::ByAttribute,
            };
            self.builder.set_proximity_precision(milli_precision);
        }

        // Typo tolerance sub-object
        if let Some(ref typo) = settings.typo_tolerance {
            if let Some(enabled) = typo.enabled {
                self.builder.set_authorize_typos(enabled);
            }
            if let Some(ref min_sizes) = typo.min_word_size_for_typos {
                if let Some(one) = min_sizes.one_typo {
                    self.builder.set_min_word_len_one_typo(one);
                }
                if let Some(two) = min_sizes.two_typos {
                    self.builder.set_min_word_len_two_typos(two);
                }
            }
            if let Some(ref words) = typo.disable_on_words {
                self.builder.set_exact_words(words.clone());
            }
            if let Some(ref attrs) = typo.disable_on_attributes {
                let hash_set: HashSet<String> = attrs.iter().cloned().collect();
                self.builder.set_exact_attributes(hash_set);
            }
            if let Some(disable) = typo.disable_on_numbers {
                self.builder.set_disable_on_numbers(disable);
            }
        }

        // Faceting sub-object
        if let Some(ref faceting) = settings.faceting {
            if let Some(max) = faceting.max_values_per_facet {
                self.builder.set_max_values_per_facet(max);
            }
            // sort_facet_values_by is a runtime setting, not stored in milli's settings builder
        }

        // Pagination sub-object
        if let Some(ref pagination) = settings.pagination {
            if let Some(max) = pagination.max_total_hits {
                self.builder.set_pagination_max_total_hits(max);
            }
        }

        if let Some(ref embedders) = settings.embedders {
            let milli_embedders: BTreeMap<String, Setting<MilliEmbeddingSettings>> = embedders
                .iter()
                .map(|(name, settings)| (name.clone(), settings.clone().into()))
                .collect();
            self.builder.set_embedder_settings(milli_embedders);
        }

        if let Some(ms) = settings.search_cutoff_ms {
            self.builder.set_search_cutoff(ms);
        }

        if let Some(ref rules) = settings.localized_attributes {
            let milli_rules: Vec<milli::LocalizedAttributesRule> = rules
                .iter()
                .map(|rule| {
                    milli::LocalizedAttributesRule::new(
                        rule.attribute_patterns.clone(),
                        rule.locales
                            .iter()
                            .filter_map(|l| {
                                // Language derives Deserialize but not FromStr;
                                // use serde_json to convert the string to a Language variant
                                let json_str = serde_json::Value::String(l.clone());
                                serde_json::from_value::<milli::tokenizer::Language>(json_str).ok()
                            })
                            .collect(),
                    )
                })
                .collect();
            self.builder.set_localized_attributes_rules(milli_rules);
        }

        if let Some(enabled) = settings.facet_search {
            self.builder.set_facet_search(enabled);
        }

        if let Some(ref mode) = settings.prefix_search {
            // Convert string to milli's PrefixSearch enum
            match mode.as_str() {
                "indexingTime" => {
                    self.builder.set_prefix_search(milli::index::PrefixSearch::IndexingTime);
                }
                "disabled" => {
                    self.builder.set_prefix_search(milli::index::PrefixSearch::Disabled);
                }
                other => {
                    return Err(Error::Internal(format!(
                        "Unknown prefix search mode '{other}'; valid values are \"indexingTime\" and \"disabled\""
                    )));
                }
            }
        }

        Ok(self.builder)
    }
}

/// Read settings from a milli index
pub(crate) fn read_settings_from_index(
    rtxn: &milli::heed::RoTxn<'_>,
    index: &milli::Index,
) -> crate::core::Result<Settings> {
    let displayed_attributes = index
        .displayed_fields(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|fields| fields.into_iter().map(|s| s.to_string()).collect());

    let searchable_attributes = index
        .searchable_fields(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .into_iter()
        .map(|s| s.into_owned())
        .collect();

    let filterable_attributes = index
        .filterable_attributes_rules(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .into_iter()
        .filter_map(|rule| match rule {
            FilterableAttributesRule::Field(name) => Some(name),
            _ => None,
        })
        .collect();

    // milli returns HashSet<String>, convert to BTreeSet for deterministic ordering
    let sortable_attributes: BTreeSet<String> = index
        .sortable_fields(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .into_iter()
        .collect();

    let ranking_rules: Vec<String> = index
        .criteria(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .into_iter()
        .map(|c| c.to_string())
        .collect();

    let stop_words: Option<BTreeSet<String>> = index
        .stop_words(rtxn)
        .map_err(crate::core::Error::Milli)?
        .map(|fst| {
            fst.stream()
                .into_strs()
                .unwrap_or_default()
                .into_iter()
                .collect()
        });

    let non_separator_tokens = index
        .non_separator_tokens(rtxn)
        .map_err(crate::core::Error::Milli)?;

    let separator_tokens = index
        .separator_tokens(rtxn)
        .map_err(crate::core::Error::Milli)?;

    let dictionary = index
        .dictionary(rtxn)
        .map_err(crate::core::Error::Milli)?;

    let synonyms_raw = index.synonyms(rtxn).map_err(|e| crate::core::Error::Heed(e))?;
    let synonyms: Option<BTreeMap<String, Vec<String>>> = if synonyms_raw.is_empty() {
        None
    } else {
        Some(
            synonyms_raw
                .into_iter()
                .map(|(k, v)| (k.join(" "), v.into_iter().map(|s| s.join(" ")).collect()))
                .collect(),
        )
    };

    let distinct_attribute = index
        .distinct_field(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|s| s.to_string());

    // Read proximity precision
    let proximity_precision = index
        .proximity_precision(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|p| match p {
            milli::proximity::ProximityPrecision::ByWord => ProximityPrecision::ByWord,
            milli::proximity::ProximityPrecision::ByAttribute => ProximityPrecision::ByAttribute,
        });

    // Assemble typo tolerance sub-object
    let typo_enabled = index.authorize_typos(rtxn).map_err(|e| crate::core::Error::Heed(e))?;
    let one_typo = index.min_word_len_one_typo(rtxn).map_err(|e| crate::core::Error::Heed(e))?;
    let two_typos = index.min_word_len_two_typos(rtxn).map_err(|e| crate::core::Error::Heed(e))?;

    let exact_words: Option<BTreeSet<String>> = index
        .exact_words(rtxn)
        .map_err(crate::core::Error::Milli)?
        .map(|fst| {
            fst.stream()
                .into_strs()
                .unwrap_or_default()
                .into_iter()
                .collect()
        });

    let exact_attrs: Vec<String> = index
        .exact_attributes(rtxn)
        .map_err(crate::core::Error::Milli)?
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let disable_on_attributes = if exact_attrs.is_empty() {
        None
    } else {
        Some(exact_attrs.into_iter().collect())
    };

    let disabled_typos_terms = index.disabled_typos_terms(rtxn).map_err(|e| crate::core::Error::Heed(e))?;

    let typo_tolerance = Some(TypoToleranceSettings {
        enabled: Some(typo_enabled),
        min_word_size_for_typos: Some(MinWordSizeForTypos {
            one_typo: Some(one_typo),
            two_typos: Some(two_typos),
        }),
        disable_on_words: exact_words,
        disable_on_attributes,
        disable_on_numbers: Some(disabled_typos_terms.disable_on_numbers),
    });

    // Assemble faceting sub-object
    let max_values_per_facet = index
        .max_values_per_facet(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|v| v as usize);

    let faceting = Some(FacetingSettings {
        max_values_per_facet,
        sort_facet_values_by: None, // milli does not directly expose this in the index reader
    });

    // Assemble pagination sub-object
    let pagination_max_total_hits = index
        .pagination_max_total_hits(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|v| v as usize);

    let pagination = Some(PaginationSettings {
        max_total_hits: pagination_max_total_hits,
    });

    let search_cutoff_ms = index.search_cutoff(rtxn).map_err(crate::core::Error::Milli)?;

    // Read embedder configurations
    let embedders = read_embedders_from_index(rtxn, index)?;

    // Read localized attributes rules
    let localized_attributes = index
        .localized_attributes_rules(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|rules| {
            rules
                .into_iter()
                .map(|rule| LocalizedAttributeRule {
                    attribute_patterns: rule.attribute_patterns.patterns,
                    locales: rule.locales.into_iter().filter_map(|l| {
                        // Language derives Serialize with a string representation.
                        // Serialize directly to a JSON string and strip the quotes.
                        serde_json::to_string(&l).ok().map(|s| s.trim_matches('"').to_string())
                    }).collect(),
                })
                .collect()
        });

    // Read facet search
    let facet_search = Some(index.facet_search(rtxn).map_err(|e| crate::core::Error::Heed(e))?);

    // Read prefix search
    let prefix_search = index
        .prefix_search(rtxn)
        .map_err(|e| crate::core::Error::Heed(e))?
        .map(|p| match p {
            milli::index::PrefixSearch::IndexingTime => "indexingTime".to_string(),
            milli::index::PrefixSearch::Disabled => "disabled".to_string(),
        });

    Ok(Settings {
        displayed_attributes,
        searchable_attributes: Some(searchable_attributes),
        filterable_attributes: Some(filterable_attributes),
        sortable_attributes: Some(sortable_attributes),
        ranking_rules: Some(ranking_rules),
        stop_words,
        non_separator_tokens,
        separator_tokens,
        dictionary,
        synonyms,
        distinct_attribute,
        proximity_precision,
        typo_tolerance,
        faceting,
        pagination,
        embedders,
        search_cutoff_ms,
        localized_attributes,
        facet_search,
        prefix_search,
    })
}

fn read_embedders_from_index(
    rtxn: &milli::heed::RoTxn<'_>,
    index: &milli::Index,
) -> crate::core::Result<Option<HashMap<String, EmbedderSettings>>> {
    let configs = index.embedding_configs();
    let embedding_configs = configs.embedding_configs(rtxn).map_err(|e| crate::core::Error::Heed(e))?;

    if embedding_configs.is_empty() {
        return Ok(None);
    }

    let embedders: HashMap<String, EmbedderSettings> = embedding_configs
        .into_iter()
        .map(|config| {
            let milli_settings: MilliEmbeddingSettings = config.config.into();
            (config.name, milli_settings.into())
        })
        .collect();

    Ok(Some(embedders))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedder_settings_openai() {
        let embedder = EmbedderSettings::openai("test-key");
        assert_eq!(embedder.source, Some(EmbedderSource::OpenAi));
        assert_eq!(embedder.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_embedder_settings_ollama() {
        let embedder = EmbedderSettings::ollama("http://localhost:11434", "nomic-embed-text");
        assert_eq!(embedder.source, Some(EmbedderSource::Ollama));
        assert_eq!(embedder.url, Some("http://localhost:11434".to_string()));
        assert_eq!(embedder.model, Some("nomic-embed-text".to_string()));
    }

    #[test]
    fn test_embedder_settings_user_provided() {
        let embedder = EmbedderSettings::user_provided(384);
        assert_eq!(embedder.source, Some(EmbedderSource::UserProvided));
        assert_eq!(embedder.dimensions, Some(384));
    }

    #[test]
    fn test_settings_builder() {
        let settings = Settings::new()
            .with_searchable_attributes(vec!["title".to_string(), "description".to_string()])
            .with_filterable_attributes(vec!["category".to_string()])
            .with_embedder("default", EmbedderSettings::openai("key"));

        assert_eq!(
            settings.searchable_attributes,
            Some(vec!["title".to_string(), "description".to_string()])
        );
        assert_eq!(
            settings.filterable_attributes,
            Some(vec!["category".to_string()])
        );
        assert!(settings.embedders.is_some());
    }

    #[test]
    fn test_settings_builder_new_fields() {
        let settings = Settings::new()
            .with_typo_tolerance(TypoToleranceSettings {
                enabled: Some(false),
                min_word_size_for_typos: Some(MinWordSizeForTypos {
                    one_typo: Some(6),
                    two_typos: Some(10),
                }),
                disable_on_words: Some(["exact".to_string()].into_iter().collect()),
                disable_on_attributes: None,
                disable_on_numbers: None,
            })
            .with_faceting(FacetingSettings {
                max_values_per_facet: Some(200),
                sort_facet_values_by: None,
            })
            .with_pagination(PaginationSettings {
                max_total_hits: Some(5000),
            })
            .with_proximity_precision(ProximityPrecision::ByAttribute)
            .with_non_separator_tokens(["@".to_string()].into_iter().collect())
            .with_separator_tokens(["|".to_string()].into_iter().collect())
            .with_dictionary(["NYC".to_string()].into_iter().collect())
            .with_facet_search(true)
            .with_prefix_search("disabled");

        assert_eq!(settings.typo_tolerance.as_ref().unwrap().enabled, Some(false));
        assert_eq!(
            settings.typo_tolerance.as_ref().unwrap().min_word_size_for_typos.as_ref().unwrap().one_typo,
            Some(6)
        );
        assert_eq!(settings.faceting.as_ref().unwrap().max_values_per_facet, Some(200));
        assert_eq!(settings.pagination.as_ref().unwrap().max_total_hits, Some(5000));
        assert_eq!(settings.proximity_precision, Some(ProximityPrecision::ByAttribute));
        assert!(settings.non_separator_tokens.as_ref().unwrap().contains("@"));
        assert!(settings.separator_tokens.as_ref().unwrap().contains("|"));
        assert!(settings.dictionary.as_ref().unwrap().contains("NYC"));
        assert_eq!(settings.facet_search, Some(true));
        assert_eq!(settings.prefix_search, Some("disabled".to_string()));
    }

    #[test]
    fn test_settings_builder_sortable_btreeset() {
        let attrs: BTreeSet<String> = ["price", "date"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let settings = Settings::new().with_sortable_attributes(attrs.clone());
        assert_eq!(settings.sortable_attributes, Some(attrs));
    }

    #[test]
    fn test_settings_builder_stop_words_btreeset() {
        let words: BTreeSet<String> = ["the", "a", "an"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let settings = Settings::new().with_stop_words(words.clone());
        assert_eq!(settings.stop_words, Some(words));
    }

    #[test]
    fn test_settings_builder_synonyms_btreemap() {
        let mut synonyms = BTreeMap::new();
        synonyms.insert("car".to_string(), vec!["automobile".to_string(), "vehicle".to_string()]);
        let settings = Settings::new().with_synonyms(synonyms.clone());
        assert_eq!(settings.synonyms, Some(synonyms));
    }

    #[test]
    fn test_settings_localized_attributes() {
        let rules = vec![LocalizedAttributeRule {
            attribute_patterns: vec!["title_*".to_string()],
            locales: vec!["eng".to_string(), "fra".to_string()],
        }];
        let settings = Settings::new().with_localized_attributes(rules);
        let rules = settings.localized_attributes.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].attribute_patterns, vec!["title_*".to_string()]);
        assert_eq!(rules[0].locales, vec!["eng".to_string(), "fra".to_string()]);
    }

    #[test]
    fn test_settings_json_roundtrip() {
        let settings = Settings::new()
            .with_searchable_attributes(vec!["title".to_string()])
            .with_typo_tolerance(TypoToleranceSettings {
                enabled: Some(true),
                min_word_size_for_typos: Some(MinWordSizeForTypos {
                    one_typo: Some(5),
                    two_typos: Some(9),
                }),
                disable_on_words: None,
                disable_on_attributes: None,
                disable_on_numbers: None,
            })
            .with_pagination(PaginationSettings { max_total_hits: Some(1000) });

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.searchable_attributes, settings.searchable_attributes);
        assert_eq!(
            deserialized.typo_tolerance.as_ref().unwrap().enabled,
            settings.typo_tolerance.as_ref().unwrap().enabled,
        );
        assert_eq!(
            deserialized.pagination.as_ref().unwrap().max_total_hits,
            settings.pagination.as_ref().unwrap().max_total_hits,
        );
    }

    #[test]
    fn test_embedder_source_conversion() {
        let source = EmbedderSource::OpenAi;
        let milli_source: milli::vector::settings::EmbedderSource = source.into();
        assert!(matches!(milli_source, milli::vector::settings::EmbedderSource::OpenAi));

        let back: EmbedderSource = milli_source.into();
        assert_eq!(back, EmbedderSource::OpenAi);
    }

    #[test]
    fn test_proximity_precision_serde() {
        let json = serde_json::json!("byWord");
        let precision: ProximityPrecision = serde_json::from_value(json).unwrap();
        assert_eq!(precision, ProximityPrecision::ByWord);

        let json = serde_json::json!("byAttribute");
        let precision: ProximityPrecision = serde_json::from_value(json).unwrap();
        assert_eq!(precision, ProximityPrecision::ByAttribute);
    }

    #[test]
    fn test_facet_values_sort_serde() {
        let json = serde_json::json!("alpha");
        let sort: FacetValuesSort = serde_json::from_value(json).unwrap();
        assert_eq!(sort, FacetValuesSort::Alpha);

        let json = serde_json::json!("count");
        let sort: FacetValuesSort = serde_json::from_value(json).unwrap();
        assert_eq!(sort, FacetValuesSort::Count);
    }
}
