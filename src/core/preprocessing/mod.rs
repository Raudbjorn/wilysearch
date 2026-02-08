//! Query preprocessing pipeline for Meilisearch-lib.
//!
//! This module provides a complete query preprocessing pipeline that includes:
//! - **Typo correction**: Using SymSpell algorithm with Meilisearch-compatible tolerance rules
//! - **Synonym expansion**: Bidirectional synonym mapping with multi-way and one-way support
//! - **Query normalization**: Preparing queries for both keyword and vector search
//!
//! # Architecture
//!
//! The preprocessing pipeline follows a staged approach:
//!
//! ```text
//! Raw Query
//!     │
//!     ▼
//! ┌─────────────────┐
//! │ Typo Correction │  "seach engne" → "search engine"
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │ Synonym Expand  │  "search engine" → ["search engine", "search tool", ...]
//! └────────┬────────┘
//!          │
//!          ▼
//! ProcessedQuery {
//!     original: "seach engne",
//!     corrected: "search engine",
//!     corrections: [("seach", "search"), ("engne", "engine")],
//!     expanded: ExpandedQuery { ... },
//!     text_for_embedding: "search engine"  // NOT expanded
//! }
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use wilysearch::core::preprocessing::{QueryPipeline, TypoCorrector, SynonymMap};
//!
//! // Build the pipeline
//! let mut typo_corrector = TypoCorrector::with_defaults()?;
//! typo_corrector.load_dictionary_from_file("dictionary.txt")?;
//!
//! let mut synonym_map = SynonymMap::new();
//! synonym_map.add_multi_way(&["hp", "hit points", "health"]);
//!
//! let pipeline = QueryPipeline::new(typo_corrector, synonym_map);
//!
//! // Process a query
//! let processed = pipeline.process("hp recovry");
//! // processed.corrected = "hp recovery"
//! // processed.expanded contains synonym alternatives
//! // processed.text_for_embedding = "hp recovery" (for vector search)
//! ```

pub mod config;
pub mod defaults;
pub mod dictionary;
pub mod error;
pub mod fts;
pub mod synonyms;
pub mod typo;

pub use config::{DictionaryPaths, NormalizationConfig, PreprocessingConfig, PreprocessingConfigBuilder};
pub use defaults::build_default_ttrpg_synonyms;
pub use dictionary::{DictionaryConfig, DictionaryGenerator, DictionaryStats};
pub use error::{PreprocessingError, Result};
pub use synonyms::{
    CampaignScopedSynonyms, ExpandedQuery, ExpandedTerm,
    SynonymConfig, SynonymMap, SynonymType, TermAlternatives,
};
pub use typo::{CorrectionRecord, TypoConfig, TypoCorrector};

use serde::{Deserialize, Serialize};

/// A fully processed query ready for search execution.
///
/// Contains all stages of preprocessing results, allowing the search engine
/// to use different representations for different purposes:
/// - `corrected`: For display in "Showing results for..." UI
/// - `corrections`: For "Did you mean?" suggestions
/// - `expanded`: For keyword search with synonym support
/// - `text_for_embedding`: For vector/semantic search (NOT expanded to preserve meaning)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessedQuery {
    /// The original, unmodified query string.
    pub original: String,

    /// The query after typo correction (before synonym expansion).
    pub corrected: String,

    /// List of corrections made during typo correction.
    /// Each entry is (original_word, corrected_word).
    /// Useful for "Did you mean X?" UI elements.
    pub corrections: Vec<CorrectionRecord>,

    /// The query with synonyms expanded at each position.
    /// Use this for keyword/full-text search.
    pub expanded: ExpandedQuery,

    /// The corrected text suitable for embedding generation.
    /// This is the corrected query WITHOUT synonym expansion,
    /// preserving the semantic meaning for vector search.
    pub text_for_embedding: String,

    /// Processing time in microseconds.
    /// Useful for performance monitoring and debugging.
    #[serde(default)]
    pub processing_time_us: u64,

    /// User-facing hints about the preprocessing.
    /// Examples: "Showing results for: X", "Y also matches: Z"
    #[serde(default)]
    pub hints: Vec<String>,
}

impl ProcessedQuery {
    /// Create a new processed query with all fields.
    pub fn new(
        original: impl Into<String>,
        corrected: impl Into<String>,
        corrections: Vec<CorrectionRecord>,
        expanded: ExpandedQuery,
        text_for_embedding: impl Into<String>,
    ) -> Self {
        Self {
            original: original.into(),
            corrected: corrected.into(),
            corrections,
            expanded,
            text_for_embedding: text_for_embedding.into(),
            processing_time_us: 0,
            hints: Vec::new(),
        }
    }

    /// Create a processed query with timing and hints.
    pub fn with_timing(
        original: impl Into<String>,
        corrected: impl Into<String>,
        corrections: Vec<CorrectionRecord>,
        expanded: ExpandedQuery,
        text_for_embedding: impl Into<String>,
        processing_time_us: u64,
        hints: Vec<String>,
    ) -> Self {
        Self {
            original: original.into(),
            corrected: corrected.into(),
            corrections,
            expanded,
            text_for_embedding: text_for_embedding.into(),
            processing_time_us,
            hints,
        }
    }

    /// Check if any typo corrections were made.
    pub fn has_corrections(&self) -> bool {
        !self.corrections.is_empty()
    }

    /// Check if any synonyms were expanded.
    pub fn has_expansions(&self) -> bool {
        self.expanded.has_expansions
    }

    /// Get the query text best suited for a given search type.
    pub fn text_for_search(&self, search_type: SearchType) -> &str {
        match search_type {
            SearchType::Keyword => &self.corrected,
            SearchType::Vector => &self.text_for_embedding,
            SearchType::Hybrid => &self.corrected,
        }
    }

    /// Generate a "Did you mean?" suggestion string.
    ///
    /// Returns `None` if no corrections were made.
    pub fn did_you_mean(&self) -> Option<String> {
        if self.corrections.is_empty() {
            None
        } else {
            Some(self.corrected.clone())
        }
    }

    /// Generate hints from corrections and expansions.
    pub fn generate_hints(&self) -> Vec<String> {
        let mut hints = Vec::new();

        // Add correction hint
        if self.has_corrections() {
            hints.push(format!("Showing results for: \"{}\"", self.corrected));
        }

        // Add expansion hints
        for group in &self.expanded.term_groups {
            if group.has_expansions() {
                if let Some(original) = group.original() {
                    let expansions: Vec<&str> = group
                        .alternatives
                        .iter()
                        .filter(|t| !t.is_original)
                        .map(|t| t.term.as_str())
                        .collect();
                    if !expansions.is_empty() {
                        hints.push(format!(
                            "\"{}\" also matches: {}",
                            original,
                            expansions.join(", ")
                        ));
                    }
                }
            }
        }

        hints
    }
}

/// Information about a single expansion applied during preprocessing.
///
/// This provides details about which term was expanded and what it expanded to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpansionInfo {
    /// The original term that was expanded.
    pub original: String,
    /// The terms it was expanded to (excluding the original).
    pub expanded_to: Vec<String>,
    /// Category of expansion (e.g., "synonym", "abbreviation", "dice").
    pub category: String,
}

/// A comprehensive preprocessing result suitable for API responses.
///
/// This is a convenience type that bundles all preprocessing information
/// in a format suitable for returning to clients or using in search pipelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreprocessingResult {
    /// Original query string.
    pub original_query: String,
    /// Processed query (corrected, suitable for search).
    pub processed_query: String,
    /// Text suitable for embedding generation.
    pub embedding_text: String,
    /// Whether any corrections were made.
    pub has_corrections: bool,
    /// Whether any expansions were applied.
    pub has_expansions: bool,
    /// Individual correction records.
    pub corrections: Vec<CorrectionRecord>,
    /// Information about applied expansions.
    pub expansions: Vec<ExpansionInfo>,
    /// User-facing hints.
    pub hints: Vec<String>,
    /// Processing time in microseconds.
    pub processing_time_us: u64,
}

impl PreprocessingResult {
    /// Create a PreprocessingResult from a ProcessedQuery.
    pub fn from_processed_query(query: ProcessedQuery) -> Self {
        let has_corrections = query.has_corrections();
        let has_expansions = query.has_expansions();
        let hints = if query.hints.is_empty() {
            query.generate_hints()
        } else {
            query.hints.clone()
        };

        // Extract expansion info from term groups
        let expansions: Vec<ExpansionInfo> = query
            .expanded
            .term_groups
            .iter()
            .filter(|group| group.has_expansions())
            .filter_map(|group| {
                group.original().map(|original| {
                    let expanded_to: Vec<String> = group
                        .alternatives
                        .iter()
                        .filter(|t| !t.is_original)
                        .map(|t| t.term.clone())
                        .collect();
                    ExpansionInfo {
                        original: original.to_string(),
                        expanded_to,
                        category: "synonym".to_string(),
                    }
                })
            })
            .collect();

        Self {
            original_query: query.original,
            processed_query: query.corrected,
            embedding_text: query.text_for_embedding,
            has_corrections,
            has_expansions,
            corrections: query.corrections,
            expansions,
            hints,
            processing_time_us: query.processing_time_us,
        }
    }

    /// Get "Did you mean?" suggestion if corrections were made.
    pub fn did_you_mean(&self) -> Option<&str> {
        if self.has_corrections {
            Some(&self.processed_query)
        } else {
            None
        }
    }
}

impl From<ProcessedQuery> for PreprocessingResult {
    fn from(query: ProcessedQuery) -> Self {
        Self::from_processed_query(query)
    }
}

/// The type of search being performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    /// Traditional keyword/full-text search.
    Keyword,
    /// Vector/semantic search using embeddings.
    Vector,
    /// Hybrid search combining keyword and vector.
    Hybrid,
}

/// Configuration for the query preprocessing pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineConfig {
    /// Configuration for typo correction.
    #[serde(default)]
    pub typo: TypoConfig,

    /// Configuration for synonym expansion.
    #[serde(default)]
    pub synonyms: SynonymConfig,

    /// Text normalization settings (lowercase, trim, unicode, whitespace collapse).
    #[serde(default)]
    pub normalization: NormalizationConfig,

    // --- Backwards-compatible accessors for the flattened fields ---
    // These are kept as serde aliases so existing configs still parse.

    /// Deprecated: use `normalization.unicode_normalize` instead.
    #[serde(default, alias = "normalizeUnicode")]
    #[doc(hidden)]
    normalize_unicode: Option<bool>,

    /// Deprecated: use `normalization.lowercase` instead.
    #[serde(default)]
    #[doc(hidden)]
    lowercase: Option<bool>,

    /// Deprecated: use `normalization.trim` instead.
    #[serde(default)]
    #[doc(hidden)]
    trim: Option<bool>,
}

impl PipelineConfig {
    /// Resolve the effective normalization config, applying any legacy field overrides.
    pub fn effective_normalization(&self) -> NormalizationConfig {
        let mut norm = self.normalization.clone();
        if let Some(v) = self.normalize_unicode {
            norm.unicode_normalize = v;
        }
        if let Some(v) = self.lowercase {
            norm.lowercase = v;
        }
        if let Some(v) = self.trim {
            norm.trim = v;
        }
        norm
    }
}

/// The query preprocessing pipeline.
///
/// Combines typo correction and synonym expansion into a single processing step.
/// The pipeline is designed to be reusable across many queries.
#[derive(Debug)]
pub struct QueryPipeline {
    /// The typo corrector instance.
    typo_corrector: TypoCorrector,

    /// The synonym map for expansion.
    synonym_map: SynonymMap,

    /// Pipeline configuration.
    config: PipelineConfig,
}

impl QueryPipeline {
    /// Create a new query pipeline with the given components.
    pub fn new(typo_corrector: TypoCorrector, synonym_map: SynonymMap) -> Self {
        Self {
            typo_corrector,
            synonym_map,
            config: PipelineConfig::default(),
        }
    }

    /// Create a new query pipeline with custom configuration.
    pub fn with_config(
        typo_corrector: TypoCorrector,
        synonym_map: SynonymMap,
        config: PipelineConfig,
    ) -> Self {
        Self {
            typo_corrector,
            synonym_map,
            config,
        }
    }

    /// Create a minimal pipeline without typo correction or synonyms.
    ///
    /// Useful for testing or when preprocessing is not needed.
    ///
    /// # Panics
    ///
    /// Panics if the SymSpell engine fails to initialize with default disabled config.
    /// This should never happen in practice.
    pub fn passthrough() -> Self {
        Self {
            typo_corrector: TypoCorrector::new(TypoConfig::default().disabled())
                .expect("default disabled TypoConfig should always build"),
            synonym_map: SynonymMap::with_config(SynonymConfig {
                enabled: false,
                ..Default::default()
            }),
            config: PipelineConfig::default(),
        }
    }

    /// Get a reference to the typo corrector.
    pub fn typo_corrector(&self) -> &TypoCorrector {
        &self.typo_corrector
    }

    /// Get a mutable reference to the typo corrector.
    pub fn typo_corrector_mut(&mut self) -> &mut TypoCorrector {
        &mut self.typo_corrector
    }

    /// Get a reference to the synonym map.
    pub fn synonym_map(&self) -> &SynonymMap {
        &self.synonym_map
    }

    /// Get a mutable reference to the synonym map.
    pub fn synonym_map_mut(&mut self) -> &mut SynonymMap {
        &mut self.synonym_map
    }

    /// Get the pipeline configuration.
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Process a raw query through the full pipeline.
    ///
    /// The pipeline performs the following steps:
    /// 1. Normalization (trim, lowercase, unicode normalization if configured)
    /// 2. Typo correction
    /// 3. Synonym expansion
    ///
    /// # Arguments
    /// * `raw_query` - The original user query string
    ///
    /// # Returns
    /// A `ProcessedQuery` containing all preprocessing results, including
    /// processing time and generated hints.
    pub fn process(&self, raw_query: &str) -> ProcessedQuery {
        let start = std::time::Instant::now();

        // Step 1: Normalize the query
        let normalized = self.normalize(raw_query);

        // Step 2: Apply typo correction
        let (corrected, corrections) = self.typo_corrector.correct_query(&normalized);

        // Step 3: Apply synonym expansion to the corrected query
        let expanded = self.synonym_map.expand_query(&corrected);

        // The text for embedding is the corrected query (NOT expanded)
        // to preserve semantic meaning for vector search
        let text_for_embedding = corrected.clone();

        let processing_time_us = start.elapsed().as_micros() as u64;

        // Build the processed query
        let mut query = ProcessedQuery::new(
            raw_query,
            corrected,
            corrections,
            expanded,
            text_for_embedding,
        );

        // Set timing and generate hints
        query.processing_time_us = processing_time_us;
        query.hints = query.generate_hints();

        query
    }

    /// Process a query and return a `PreprocessingResult` for API responses.
    ///
    /// This is a convenience method that returns a flat structure suitable
    /// for serialization and API responses.
    pub fn process_for_api(&self, raw_query: &str) -> PreprocessingResult {
        self.process(raw_query).into()
    }

    /// Process a query, returning only the corrected text (no expansion).
    ///
    /// Useful when you only need typo correction.
    pub fn correct(&self, raw_query: &str) -> (String, Vec<CorrectionRecord>) {
        let normalized = self.normalize(raw_query);
        self.typo_corrector.correct_query(&normalized)
    }

    /// Process a query, returning only the expanded query (no typo correction).
    ///
    /// Useful when you only need synonym expansion.
    pub fn expand(&self, raw_query: &str) -> ExpandedQuery {
        let normalized = self.normalize(raw_query);
        self.synonym_map.expand_query(&normalized)
    }

    /// Normalize a query string according to pipeline configuration.
    ///
    /// Applies transformations in order: trim, lowercase, unicode NFKC,
    /// whitespace collapse. Each enabled step reuses the same `String`
    /// buffer where possible to avoid extra allocations.
    fn normalize(&self, query: &str) -> String {
        let norm = self.config.effective_normalization();

        // Start with a Cow to avoid allocating when no transforms are needed.
        let trimmed = if norm.trim { query.trim() } else { query };

        // Lowercase always produces a new String (may change byte length for
        // non-ASCII), so we build from here when enabled.
        let mut result = if norm.lowercase {
            trimmed.to_lowercase()
        } else {
            trimmed.to_string()
        };

        if norm.unicode_normalize {
            result = unicode_normalization_nfkc(&result);
        }

        if norm.collapse_whitespace {
            result = collapse_whitespace(&result);
        }

        result
    }
}

/// Perform NFKC normalization on a string.
///
/// NFKC (Normalization Form Compatibility Composition) normalizes compatibility
/// characters to their canonical equivalents (e.g., full-width characters,
/// ligatures, superscripts).
fn unicode_normalization_nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

/// Collapse multiple whitespace characters into single spaces.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_whitespace = false;

    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_was_whitespace {
                result.push(' ');
                prev_was_whitespace = true;
            }
        } else {
            result.push(c);
            prev_was_whitespace = false;
        }
    }

    result
}

/// Builder for constructing a `QueryPipeline` with a fluent interface.
#[derive(Debug, Default)]
pub struct QueryPipelineBuilder {
    typo_config: TypoConfig,
    synonym_map: SynonymMap,
    pipeline_config: PipelineConfig,
    dictionary_entries: Vec<(String, i64)>,
    word_list: Vec<String>,
    protected_words: Vec<String>,
}

impl QueryPipelineBuilder {
    /// Create a new pipeline builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the typo correction configuration.
    pub fn typo_config(mut self, config: TypoConfig) -> Self {
        self.typo_config = config;
        self
    }

    /// Set the minimum word size for single-typo tolerance.
    pub fn min_word_size_one_typo(mut self, size: usize) -> Self {
        self.typo_config.min_word_size_one_typo = size;
        self
    }

    /// Set the minimum word size for two-typo tolerance.
    pub fn min_word_size_two_typos(mut self, size: usize) -> Self {
        self.typo_config.min_word_size_two_typos = size;
        self
    }

    /// Disable typo correction.
    pub fn disable_typo_correction(mut self) -> Self {
        self.typo_config.enabled = false;
        self
    }

    /// Add words that should not be corrected.
    pub fn protected_words<I, S>(mut self, words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.protected_words.extend(words.into_iter().map(Into::into));
        self
    }

    /// Add dictionary entries for typo correction.
    pub fn dictionary_entries<I, S>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (S, i64)>,
        S: Into<String>,
    {
        self.dictionary_entries.extend(entries.into_iter().map(|(w, f)| (w.into(), f)));
        self
    }

    /// Add words to the dictionary with default frequency.
    pub fn word_list<I, S>(mut self, words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.word_list.extend(words.into_iter().map(Into::into));
        self
    }

    /// Set the synonym map.
    pub fn synonym_map(mut self, map: SynonymMap) -> Self {
        self.synonym_map = map;
        self
    }

    /// Add a multi-way synonym group.
    pub fn multi_way_synonyms<I, S>(mut self, terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.synonym_map.add_multi_way(terms);
        self
    }

    /// Add a one-way synonym mapping.
    pub fn one_way_synonyms<S, I, T>(mut self, source: S, targets: I) -> Self
    where
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        self.synonym_map.add_one_way(source, targets);
        self
    }

    /// Set the maximum number of synonym expansions per term.
    pub fn max_expansions(mut self, max: usize) -> Self {
        self.synonym_map.config.max_expansions = max;
        self
    }

    /// Disable synonym expansion.
    pub fn disable_synonym_expansion(mut self) -> Self {
        self.synonym_map.config.enabled = false;
        self
    }

    /// Enable Unicode normalization.
    pub fn normalize_unicode(mut self) -> Self {
        self.pipeline_config.normalization.unicode_normalize = true;
        self
    }

    /// Enable lowercase conversion.
    pub fn lowercase(mut self) -> Self {
        self.pipeline_config.normalization.lowercase = true;
        self
    }

    /// Build the query pipeline.
    pub fn build(self) -> Result<QueryPipeline> {
        let mut typo_corrector = TypoCorrector::new(self.typo_config)?;

        // Load dictionary entries
        if !self.dictionary_entries.is_empty() {
            typo_corrector.load_dictionary(
                self.dictionary_entries.iter().map(|(w, f)| (w.as_str(), *f))
            );
        }

        // Load word list
        if !self.word_list.is_empty() {
            typo_corrector.load_word_list(self.word_list.iter().map(|s| s.as_str()));
        }

        // Add protected words
        if !self.protected_words.is_empty() {
            typo_corrector.add_protected_words(self.protected_words.iter().map(|s| s.as_str()));
        }

        Ok(QueryPipeline::with_config(typo_corrector, self.synonym_map, self.pipeline_config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_pipeline() {
        let pipeline = QueryPipeline::passthrough();
        let processed = pipeline.process("hello world");

        assert_eq!(processed.original, "hello world");
        assert_eq!(processed.corrected, "hello world");
        assert!(processed.corrections.is_empty());
        assert_eq!(processed.text_for_embedding, "hello world");
    }

    #[test]
    fn test_pipeline_with_synonyms() {
        let mut synonym_map = SynonymMap::new();
        synonym_map.add_multi_way(&["hp", "hit points", "health"]);

        let pipeline = QueryPipeline::new(
            TypoCorrector::new(TypoConfig::default().disabled()).unwrap(),
            synonym_map,
        );

        let processed = pipeline.process("hp recovery");

        assert!(processed.has_expansions());
        assert_eq!(processed.text_for_embedding, "hp recovery");

        // Check that expanded query has alternatives
        let fts5 = processed.expanded.to_fts5_match();
        assert!(fts5.contains("OR"));
    }

    #[test]
    fn test_builder_pattern() {
        let pipeline = QueryPipelineBuilder::new()
            .min_word_size_one_typo(4)
            .min_word_size_two_typos(8)
            .multi_way_synonyms(&["hp", "health"])
            .one_way_synonyms("dragon", &["wyrm"])
            .max_expansions(5)
            .build()
            .unwrap();

        assert_eq!(pipeline.typo_corrector().config().min_word_size_one_typo, 4);
        assert!(pipeline.synonym_map().has_synonyms("hp"));
        assert!(pipeline.synonym_map().has_synonyms("dragon"));
    }

    #[test]
    fn test_whitespace_collapse() {
        assert_eq!(collapse_whitespace("hello  world"), "hello world");
        assert_eq!(collapse_whitespace("  hello   world  "), " hello world ");
        assert_eq!(collapse_whitespace("hello\t\nworld"), "hello world");
    }

    #[test]
    fn test_search_type_text_selection() {
        let pipeline = QueryPipeline::passthrough();
        let processed = pipeline.process("test query");

        assert_eq!(processed.text_for_search(SearchType::Keyword), "test query");
        assert_eq!(processed.text_for_search(SearchType::Vector), "test query");
        assert_eq!(processed.text_for_search(SearchType::Hybrid), "test query");
    }

    #[test]
    fn test_did_you_mean() {
        // Without corrections
        let processed = ProcessedQuery::new(
            "hello",
            "hello",
            vec![],
            ExpandedQuery::new("hello"),
            "hello",
        );
        assert!(processed.did_you_mean().is_none());

        // With corrections
        let processed = ProcessedQuery::new(
            "helo",
            "hello",
            vec![CorrectionRecord::new("helo", "hello", 1)],
            ExpandedQuery::new("hello"),
            "hello",
        );
        assert_eq!(processed.did_you_mean(), Some("hello".to_string()));
    }

    #[test]
    fn test_pipeline_normalization() {
        let pipeline = QueryPipeline::with_config(
            TypoCorrector::new(TypoConfig::default().disabled()).unwrap(),
            SynonymMap::new(),
            PipelineConfig {
                normalization: NormalizationConfig {
                    trim: true,
                    lowercase: true,
                    collapse_whitespace: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let processed = pipeline.process("  HELLO  WORLD  ");
        assert_eq!(processed.corrected, "hello world");
    }
}
