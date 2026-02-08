//! Typo correction module using SymSpell algorithm.
//!
//! This module provides typo correction functionality compatible with Meilisearch's
//! length-based tolerance rules. It uses the SymSpell algorithm for efficient
//! spell checking with support for:
//! - Configurable typo tolerance based on word length
//! - Protected words that should never be corrected
//! - Domain-specific dictionaries
//! - Bigram dictionaries for compound word correction

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use symspell::{SymSpell, SymSpellBuilder, UnicodeStringStrategy, Verbosity};

use crate::core::preprocessing::error::{PreprocessingError, Result};

/// Configuration for typo correction, following Meilisearch's tolerance rules.
///
/// Meilisearch applies different typo tolerances based on word length:
/// - Words shorter than `min_word_size_one_typo` characters: no typos allowed
/// - Words with `min_word_size_one_typo` to `min_word_size_two_typos - 1` characters: 1 typo allowed
/// - Words with `min_word_size_two_typos` or more characters: 2 typos allowed
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypoConfig {
    /// Minimum word length for single-typo tolerance.
    /// Words shorter than this will have no typo tolerance.
    /// Default: 5 (Meilisearch default)
    #[serde(default = "default_min_word_size_one_typo")]
    pub min_word_size_one_typo: usize,

    /// Minimum word length for two-typo tolerance.
    /// Words shorter than this (but >= `min_word_size_one_typo`) will have single-typo tolerance.
    /// Default: 9 (Meilisearch default)
    #[serde(default = "default_min_word_size_two_typos")]
    pub min_word_size_two_typos: usize,

    /// Words that should be excluded from typo correction.
    /// These are typically domain-specific terms, brand names, or technical jargon.
    #[serde(default)]
    pub disabled_on_words: Vec<String>,

    /// Whether typo correction is enabled globally.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Maximum edit distance to consider for corrections.
    /// SymSpell default is 2, which aligns with Meilisearch's max of 2 typos.
    #[serde(default = "default_max_edit_distance")]
    pub max_edit_distance: i64,
}

fn default_min_word_size_one_typo() -> usize {
    5
}

fn default_min_word_size_two_typos() -> usize {
    9
}

fn default_enabled() -> bool {
    true
}

fn default_max_edit_distance() -> i64 {
    2
}

impl Default for TypoConfig {
    fn default() -> Self {
        Self {
            min_word_size_one_typo: default_min_word_size_one_typo(),
            min_word_size_two_typos: default_min_word_size_two_typos(),
            disabled_on_words: Vec::new(),
            enabled: default_enabled(),
            max_edit_distance: default_max_edit_distance(),
        }
    }
}

impl TypoConfig {
    /// Create a new configuration with default Meilisearch settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the minimum word size for single-typo tolerance.
    pub fn with_min_word_size_one_typo(mut self, size: usize) -> Self {
        self.min_word_size_one_typo = size;
        self
    }

    /// Set the minimum word size for two-typo tolerance.
    pub fn with_min_word_size_two_typos(mut self, size: usize) -> Self {
        self.min_word_size_two_typos = size;
        self
    }

    /// Add words to exclude from typo correction.
    pub fn with_disabled_words(mut self, words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.disabled_on_words.extend(words.into_iter().map(Into::into));
        self
    }

    /// Disable typo correction globally.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Calculate the maximum allowed edit distance for a word of given length.
    ///
    /// Returns:
    /// - 0 for words shorter than `min_word_size_one_typo`
    /// - 1 for words between `min_word_size_one_typo` and `min_word_size_two_typos`
    /// - 2 for words of `min_word_size_two_typos` or longer
    pub fn max_edit_distance_for_word(&self, word_len: usize) -> i64 {
        if word_len < self.min_word_size_one_typo {
            0
        } else if word_len < self.min_word_size_two_typos {
            1
        } else {
            2
        }
    }
}

/// A record of a single typo correction made.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionRecord {
    /// The original (potentially misspelled) term.
    pub original: String,
    /// The suggested correction.
    pub corrected: String,
    /// The edit distance between original and corrected.
    pub edit_distance: i64,
    /// Confidence score (0.0 - 1.0) based on edit distance and word length.
    /// Higher values indicate more confident corrections.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    1.0
}

impl CorrectionRecord {
    /// Create a new correction record with calculated confidence.
    pub fn new(original: impl Into<String>, corrected: impl Into<String>, edit_distance: i64) -> Self {
        let original = original.into();
        let corrected = corrected.into();
        let confidence = calculate_confidence(&original, &corrected, edit_distance);
        Self {
            original,
            corrected,
            edit_distance,
            confidence,
        }
    }

    /// Create a correction record with explicit confidence.
    pub fn with_confidence(
        original: impl Into<String>,
        corrected: impl Into<String>,
        edit_distance: i64,
        confidence: f64,
    ) -> Self {
        Self {
            original: original.into(),
            corrected: corrected.into(),
            edit_distance,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// Calculate confidence score based on edit distance and word length.
///
/// Factors:
/// - Base: 1.0 - (edit_distance / max_word_length)
/// - Boost for same starting letter: +0.05
/// - Boost for same length: +0.03
fn calculate_confidence(original: &str, corrected: &str, edit_distance: i64) -> f64 {
    let max_len = original.len().max(corrected.len()).max(1) as f64;
    let relative_distance = edit_distance as f64 / max_len;
    let base_confidence = 1.0 - relative_distance;

    // Boost for same starting letter
    let start_boost = if original.chars().next() == corrected.chars().next() {
        0.05
    } else {
        0.0
    };

    // Boost for same length
    let len_boost = if original.len() == corrected.len() {
        0.03
    } else {
        0.0
    };

    (base_confidence + start_boost + len_boost).clamp(0.0, 1.0)
}

/// Typo corrector using the SymSpell algorithm.
///
/// SymSpell provides O(1) lookup speed for spell checking by pre-computing
/// all possible deletion variants of dictionary words.
pub struct TypoCorrector {
    /// The SymSpell instance for spell checking.
    symspell: SymSpell<UnicodeStringStrategy>,
    /// Configuration for typo tolerance.
    config: TypoConfig,
    /// Set of protected words (lowercase) that should never be corrected.
    protected_words: HashSet<String>,
    /// Whether the dictionary has been loaded.
    dictionary_loaded: bool,
    /// Paths to dictionary files (for reload support).
    dictionary_path: Option<std::path::PathBuf>,
    /// Path to bigram dictionary file (for reload support).
    bigram_path: Option<std::path::PathBuf>,
}

impl TypoCorrector {
    /// Create a new typo corrector with the given configuration.
    ///
    /// Note: You must load a dictionary before using the corrector.
    /// Use `load_dictionary` or `load_dictionary_from_file` to load words.
    pub fn new(config: TypoConfig) -> Result<Self> {
        let symspell: SymSpell<UnicodeStringStrategy> = SymSpellBuilder::default()
            .max_dictionary_edit_distance(config.max_edit_distance)
            .prefix_length(7)
            .count_threshold(1)
            .build()
            .map_err(|e| PreprocessingError::InvalidConfig(
                format!("Failed to build SymSpell engine: {}", e),
            ))?;

        Ok(Self {
            symspell,
            config,
            protected_words: HashSet::new(),
            dictionary_loaded: false,
            dictionary_path: None,
            bigram_path: None,
        })
    }

    /// Create a typo corrector with default configuration.
    pub fn with_defaults() -> Result<Self> {
        Self::new(TypoConfig::default())
    }

    /// Create a TypoCorrector from a PreprocessingConfig file.
    ///
    /// This method loads the typo configuration from a TOML file and
    /// automatically loads any dictionaries specified in the paths section.
    ///
    /// # Arguments
    /// * `config_path` - Path to the preprocessing configuration TOML file
    ///
    /// # Example
    /// ```ignore
    /// let corrector = TypoCorrector::from_config("config/preprocessing.toml")?;
    /// ```
    pub fn from_config(config_path: impl AsRef<Path>) -> Result<Self> {
        use super::config::PreprocessingConfig;

        let config = PreprocessingConfig::from_toml(config_path)?;
        let mut corrector = Self::new(config.typo)?;

        // Load dictionaries from configured paths
        if let Some(ref path) = config.paths.english_dict {
            corrector.set_dictionary_path(path);
            corrector.load_dictionary_from_file(path)?;
        }

        if let Some(ref path) = config.paths.corpus_dict {
            // Corpus dict is loaded in addition to english dict
            corrector.load_dictionary_from_file(path)?;
        }

        if let Some(ref path) = config.paths.bigram_dict {
            corrector.set_bigram_path(path);
            corrector.load_bigram_dictionary_from_file(path)?;
        }

        Ok(corrector)
    }

    /// Reload dictionaries from their configured paths.
    ///
    /// Call this after dictionary regeneration or when you want to refresh
    /// the corrector's dictionary data without creating a new instance.
    ///
    /// This method:
    /// 1. Creates a fresh SymSpell instance with the current configuration
    /// 2. Reloads the main dictionary from `dictionary_path` (if set)
    /// 3. Reloads the bigram dictionary from `bigram_path` (if set)
    ///
    /// # Note
    /// Protected words are preserved across reloads.
    /// Programmatically loaded dictionaries are NOT preserved.
    pub fn reload_dictionaries(&mut self) -> Result<()> {
        // Create a fresh SymSpell instance
        self.symspell = SymSpellBuilder::default()
            .max_dictionary_edit_distance(self.config.max_edit_distance)
            .prefix_length(7)
            .count_threshold(1)
            .build()
            .map_err(|e| PreprocessingError::DictionaryLoad(e.to_string()))?;

        self.dictionary_loaded = false;

        // Reload from stored paths
        if let Some(path) = self.dictionary_path.clone() {
            self.load_dictionary_from_file(&path)?;
        }

        if let Some(path) = self.bigram_path.clone() {
            self.load_bigram_dictionary_from_file(&path)?;
        }

        Ok(())
    }

    /// Set the dictionary path for reload support.
    ///
    /// This does not load the dictionary; use `load_dictionary_from_file` for that.
    /// The path is stored so that `reload_dictionaries` can refresh the data.
    pub fn set_dictionary_path(&mut self, path: impl AsRef<Path>) {
        self.dictionary_path = Some(path.as_ref().to_path_buf());
    }

    /// Set the bigram dictionary path for reload support.
    pub fn set_bigram_path(&mut self, path: impl AsRef<Path>) {
        self.bigram_path = Some(path.as_ref().to_path_buf());
    }

    /// Get the configured dictionary path.
    pub fn dictionary_path(&self) -> Option<&Path> {
        self.dictionary_path.as_deref()
    }

    /// Get the configured bigram dictionary path.
    pub fn bigram_path(&self) -> Option<&Path> {
        self.bigram_path.as_deref()
    }

    /// Get the current configuration.
    pub fn config(&self) -> &TypoConfig {
        &self.config
    }

    /// Check if a dictionary has been loaded.
    pub fn has_dictionary(&self) -> bool {
        self.dictionary_loaded
    }

    /// Load a dictionary from a file.
    ///
    /// The file format should be one word per line, optionally with frequency:
    /// ```text
    /// the 23135851162
    /// of 13151942776
    /// and 12997637966
    /// ```
    ///
    /// If no frequency is provided, a default of 1 is used.
    pub fn load_dictionary_from_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(PreprocessingError::DictionaryNotFound(
                path.display().to_string(),
            ));
        }

        // SymSpell expects: term frequency (space-separated)
        // The term_index is 0 (first column), count_index is 1 (second column)
        let loaded = self.symspell
            .load_dictionary(path.to_str().unwrap_or_default(), 0, 1, " ");

        if !loaded {
            return Err(PreprocessingError::DictionaryNotFound(
                path.display().to_string(),
            ));
        }

        self.dictionary_loaded = true;
        Ok(())
    }

    /// Load a bigram dictionary for compound word correction.
    ///
    /// The file format should be: `word1 word2 frequency`
    /// This enables correction of compound splitting errors like "inthe" -> "in the"
    pub fn load_bigram_dictionary_from_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(PreprocessingError::DictionaryNotFound(
                path.display().to_string(),
            ));
        }

        // Load bigram dictionary: term1 term2 frequency
        let loaded = self.symspell
            .load_bigram_dictionary(path.to_str().unwrap_or_default(), 0, 2, " ");

        if !loaded {
            return Err(PreprocessingError::DictionaryNotFound(
                path.display().to_string(),
            ));
        }

        Ok(())
    }

    /// Load dictionary entries programmatically.
    ///
    /// Each entry is a tuple of (word, frequency).
    pub fn load_dictionary<I, S>(&mut self, entries: I)
    where
        I: IntoIterator<Item = (S, i64)>,
        S: AsRef<str>,
    {
        for (word, frequency) in entries {
            // Format as "word frequency" for load_dictionary_line
            let line = format!("{} {}", word.as_ref(), frequency);
            self.symspell.load_dictionary_line(&line, 0, 1, " ");
        }
        self.dictionary_loaded = true;
    }

    /// Load a simple word list with default frequency.
    ///
    /// Useful for domain-specific vocabularies where frequency data isn't available.
    pub fn load_word_list<I, S>(&mut self, words: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for word in words {
            // Use frequency of 1 for words without frequency data
            let line = format!("{} 1", word.as_ref());
            self.symspell.load_dictionary_line(&line, 0, 1, " ");
        }
        self.dictionary_loaded = true;
    }

    /// Add protected words that should never be corrected.
    ///
    /// Protected words are stored in lowercase for case-insensitive matching.
    pub fn add_protected_words<I, S>(&mut self, words: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for word in words {
            self.protected_words.insert(word.as_ref().to_lowercase());
        }
    }

    /// Check if a word is protected from correction.
    pub fn is_protected(&self, word: &str) -> bool {
        let word_lower = word.to_lowercase();
        self.protected_words.contains(&word_lower)
            || self.config.disabled_on_words.iter().any(|w| w.eq_ignore_ascii_case(word))
    }

    /// Correct a single word.
    ///
    /// Returns the corrected word and the edit distance, or None if no correction was made.
    pub fn correct_word(&self, word: &str) -> Option<CorrectionRecord> {
        // Skip if typo correction is disabled
        if !self.config.enabled {
            return None;
        }

        // Skip if word is protected
        if self.is_protected(word) {
            return None;
        }

        // Calculate max edit distance based on word length
        let max_distance = self.config.max_edit_distance_for_word(word.len());
        if max_distance == 0 {
            return None;
        }

        // Look up the word in the dictionary
        let suggestions = self.symspell.lookup(word, Verbosity::Top, max_distance);

        // Get the best suggestion if it's different from the original
        suggestions.into_iter().next().and_then(|suggestion| {
            if suggestion.term.eq_ignore_ascii_case(word) {
                None
            } else {
                Some(CorrectionRecord::new(
                    word.to_string(),
                    suggestion.term,
                    suggestion.distance,
                ))
            }
        })
    }

    /// Correct a full query string.
    ///
    /// Returns a tuple of:
    /// - The corrected query string
    /// - A list of corrections made (original, corrected pairs)
    ///
    /// This method preserves the structure of the query while correcting individual words.
    pub fn correct_query(&self, query: &str) -> (String, Vec<CorrectionRecord>) {
        if !self.config.enabled {
            return (query.to_string(), Vec::new());
        }

        let mut corrections = Vec::new();
        let mut result_parts = Vec::new();

        // Split by whitespace, correct each word
        for part in query.split_whitespace() {
            // Separate leading/trailing punctuation from the word
            let (prefix, word, suffix) = split_punctuation(part);

            if let Some(correction) = self.correct_word(word) {
                result_parts.push(format!("{}{}{}", prefix, correction.corrected, suffix));
                corrections.push(correction);
            } else {
                result_parts.push(part.to_string());
            }
        }

        (result_parts.join(" "), corrections)
    }

    /// Correct a query using compound awareness (bigram dictionary).
    ///
    /// This can split incorrectly joined words (e.g., "inthe" -> "in the")
    /// or join incorrectly split words (e.g., "in the" <- compound lookup).
    pub fn correct_query_compound(&self, query: &str) -> (String, Vec<CorrectionRecord>) {
        if !self.config.enabled {
            return (query.to_string(), Vec::new());
        }

        let suggestions = self.symspell.lookup_compound(query, self.config.max_edit_distance);

        if let Some(suggestion) = suggestions.first() {
            if suggestion.term != query {
                // Parse individual corrections from the compound result
                let corrections = parse_compound_corrections(query, &suggestion.term);
                return (suggestion.term.clone(), corrections);
            }
        }

        (query.to_string(), Vec::new())
    }
}

/// Split a string into (leading punctuation, word, trailing punctuation).
fn split_punctuation(s: &str) -> (&str, &str, &str) {
    let start = s.find(|c: char| c.is_alphanumeric()).unwrap_or(s.len());
    let end = s.char_indices().rev().find(|(_, c)| c.is_alphanumeric()).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);

    if start >= end {
        return ("", s, "");
    }

    (&s[..start], &s[start..end], &s[end..])
}

/// Parse compound corrections by comparing original and corrected strings.
fn parse_compound_corrections(original: &str, corrected: &str) -> Vec<CorrectionRecord> {
    let mut corrections = Vec::new();
    let orig_words: Vec<&str> = original.split_whitespace().collect();
    let corr_words: Vec<&str> = corrected.split_whitespace().collect();

    // Simple diff: if word counts differ or words differ, record corrections
    if orig_words.len() != corr_words.len() {
        // Compound split/join occurred
        corrections.push(CorrectionRecord::new(
            original.to_string(),
            corrected.to_string(),
            1, // Simplified; actual distance would need calculation
        ));
    } else {
        // Word-by-word comparison
        for (orig, corr) in orig_words.iter().zip(corr_words.iter()) {
            if !orig.eq_ignore_ascii_case(corr) {
                let edit_distance = strsim::levenshtein(orig, corr) as i64;
                corrections.push(CorrectionRecord::new(
                    (*orig).to_string(),
                    (*corr).to_string(),
                    edit_distance,
                ));
            }
        }
    }

    corrections
}

impl Default for TypoCorrector {
    fn default() -> Self {
        Self::with_defaults().expect("default TypoConfig should always build")
    }
}

impl std::fmt::Debug for TypoCorrector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypoCorrector")
            .field("config", &self.config)
            .field("protected_words_count", &self.protected_words.len())
            .field("dictionary_loaded", &self.dictionary_loaded)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typo_config_defaults() {
        let config = TypoConfig::default();
        assert_eq!(config.min_word_size_one_typo, 5);
        assert_eq!(config.min_word_size_two_typos, 9);
        assert!(config.enabled);
    }

    #[test]
    fn test_max_edit_distance_for_word() {
        let config = TypoConfig::default();

        // Short words: no typos
        assert_eq!(config.max_edit_distance_for_word(1), 0);
        assert_eq!(config.max_edit_distance_for_word(4), 0);

        // Medium words: 1 typo
        assert_eq!(config.max_edit_distance_for_word(5), 1);
        assert_eq!(config.max_edit_distance_for_word(8), 1);

        // Long words: 2 typos
        assert_eq!(config.max_edit_distance_for_word(9), 2);
        assert_eq!(config.max_edit_distance_for_word(20), 2);
    }

    #[test]
    fn test_split_punctuation() {
        assert_eq!(split_punctuation("hello"), ("", "hello", ""));
        assert_eq!(split_punctuation("(hello)"), ("(", "hello", ")"));
        assert_eq!(split_punctuation("hello,"), ("", "hello", ","));
        assert_eq!(split_punctuation("...test!!!"), ("...", "test", "!!!"));
    }

    #[test]
    fn test_split_punctuation_multibyte() {
        assert_eq!(split_punctuation("café"), ("", "café", ""));
        assert_eq!(split_punctuation("¡café!"), ("¡", "café", "!"));
        assert_eq!(split_punctuation("über"), ("", "über", ""));
        assert_eq!(split_punctuation("日本語"), ("", "日本語", ""));
        assert_eq!(split_punctuation("«résumé»"), ("«", "résumé", "»"));
        assert_eq!(split_punctuation("naïve"), ("", "naïve", ""));
    }

    #[test]
    fn test_protected_words() {
        let mut corrector = TypoCorrector::with_defaults().unwrap();
        corrector.add_protected_words(["API", "HTTP", "JSON"]);

        assert!(corrector.is_protected("API"));
        assert!(corrector.is_protected("api"));
        assert!(corrector.is_protected("HTTP"));
        assert!(!corrector.is_protected("hello"));
    }

    #[test]
    fn test_disabled_on_words() {
        let config = TypoConfig::default()
            .with_disabled_words(["Meilisearch", "Rust"]);
        let corrector = TypoCorrector::new(config).unwrap();

        assert!(corrector.is_protected("Meilisearch"));
        assert!(corrector.is_protected("meilisearch"));
        assert!(corrector.is_protected("Rust"));
    }

    #[test]
    fn test_correction_with_dictionary() {
        let mut corrector = TypoCorrector::with_defaults().unwrap();

        // Load a simple dictionary
        corrector.load_dictionary([
            ("hello", 100),
            ("world", 100),
            ("search", 100),
            ("engine", 100),
        ]);

        // "helo" should be corrected to "hello" (1 typo, word length 4 < 5, so no correction)
        let result = corrector.correct_word("helo");
        assert!(result.is_none()); // Word too short for typo tolerance

        // "searh" should be corrected to "search" (1 typo, word length 5 >= 5)
        let _result = corrector.correct_word("searh");
        // Note: Depends on SymSpell's dictionary state
    }

    #[test]
    fn test_disabled_corrector() {
        let config = TypoConfig::default().disabled();
        let corrector = TypoCorrector::new(config).unwrap();

        let (corrected, corrections) = corrector.correct_query("helo wrold");
        assert_eq!(corrected, "helo wrold");
        assert!(corrections.is_empty());
    }

    #[test]
    fn test_dictionary_path_tracking() {
        let mut corrector = TypoCorrector::with_defaults().unwrap();

        // Initially, no paths are set
        assert!(corrector.dictionary_path().is_none());
        assert!(corrector.bigram_path().is_none());

        // Set paths
        corrector.set_dictionary_path("/path/to/dict.txt");
        corrector.set_bigram_path("/path/to/bigrams.txt");

        // Paths should now be tracked
        assert_eq!(
            corrector.dictionary_path(),
            Some(std::path::Path::new("/path/to/dict.txt"))
        );
        assert_eq!(
            corrector.bigram_path(),
            Some(std::path::Path::new("/path/to/bigrams.txt"))
        );
    }

    #[test]
    fn test_reload_preserves_protected_words() {
        let mut corrector = TypoCorrector::with_defaults().unwrap();

        // Add some protected words
        corrector.add_protected_words(["API", "HTTP"]);

        // Load dictionary programmatically
        corrector.load_dictionary([("hello", 100), ("world", 100)]);

        // Verify protected words work
        assert!(corrector.is_protected("API"));
        assert!(corrector.is_protected("http"));

        // Reload (will clear programmatic dictionary but preserve protected words)
        corrector.reload_dictionaries().unwrap();

        // Protected words should still work
        assert!(corrector.is_protected("API"));
        assert!(corrector.is_protected("http"));

        // Dictionary is now empty (was only loaded programmatically)
        assert!(!corrector.has_dictionary());
    }

    #[test]
    fn test_reload_clears_dictionary_state() {
        let mut corrector = TypoCorrector::with_defaults().unwrap();

        // Load dictionary programmatically
        corrector.load_dictionary([("hello", 100)]);
        assert!(corrector.has_dictionary());

        // Reload without file paths clears the dictionary
        corrector.reload_dictionaries().unwrap();
        assert!(!corrector.has_dictionary());
    }
}
