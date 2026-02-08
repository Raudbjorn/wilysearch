//! Dictionary generation for corpus-aware typo correction.
//!
//! This module generates frequency dictionaries from indexed document content to ensure
//! domain-specific terms are correctly recognized by the SymSpell typo corrector.
//!
//! # Features
//!
//! - **Word frequency dictionaries**: Extracts word frequencies from document content
//! - **Domain boost**: Applies a configurable multiplier (default 10x) to corpus term frequencies
//! - **Bigram dictionaries**: Generates word pair frequencies for compound word detection
//! - **File persistence**: Stores dictionaries in SymSpell-compatible format
//!
//! # Dictionary Formats
//!
//! **Word dictionary** (SymSpell format):
//! ```text
//! word frequency
//! search 1000
//! engine 800
//! ```
//!
//! **Bigram dictionary** (SymSpell format):
//! ```text
//! word1 word2 frequency
//! search engine 500
//! full text 300
//! ```
//!
//! # Example
//!
//! ```ignore
//! use wilysearch::core::preprocessing::{DictionaryGenerator, DictionaryConfig};
//!
//! let generator = DictionaryGenerator::with_defaults();
//!
//! // Build dictionaries from document content
//! let documents = vec![
//!     "Meilisearch is a fast search engine",
//!     "Full-text search with typo tolerance",
//! ];
//!
//! let stats = generator.rebuild_all(
//!     documents.iter().map(|s| *s),
//!     "/data/corpus_dictionary.txt",
//!     "/data/bigram_dictionary.txt",
//! )?;
//!
//! println!("Generated {} unique words, {} bigrams", stats.unique_words, stats.unique_bigrams);
//! ```
//!
//! # Requirements Traceability
//!
//! This module implements REQ-QP-004 (Dictionary Generation):
//! - REQ-QP-004.1: Generate word frequency dictionaries from document content
//! - REQ-QP-004.2: Apply domain boost (10x) for corpus terms
//! - REQ-QP-004.3: Generate bigram dictionaries for compound word detection
//! - REQ-QP-004.5: Store dictionaries in specified data directory

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::preprocessing::error::Result;

/// Configuration for dictionary generation.
///
/// Controls how words are extracted from documents and how frequencies are calculated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryConfig {
    /// Frequency multiplier for domain terms (default: 10).
    ///
    /// This boost ensures domain-specific vocabulary is prioritized over
    /// general language dictionaries when merged. A value of 10 means
    /// corpus terms will have 10x their raw frequency count.
    #[serde(default = "default_domain_boost")]
    pub domain_boost: u64,

    /// Minimum word length to include (default: 2).
    ///
    /// Words shorter than this threshold are excluded from the dictionary.
    /// Single-character words are typically noise in most corpora.
    #[serde(default = "default_min_word_length")]
    pub min_word_length: usize,

    /// Minimum frequency to include in dictionary (default: 1).
    ///
    /// Words appearing fewer times than this threshold are excluded.
    /// Increase this value to filter out rare terms or potential OCR errors.
    #[serde(default = "default_min_frequency")]
    pub min_frequency: u64,

    /// Maximum word length to include (default: 50).
    ///
    /// Words longer than this are likely malformed or concatenated strings.
    #[serde(default = "default_max_word_length")]
    pub max_word_length: usize,

    /// Whether to preserve case (default: false).
    ///
    /// When false, all words are lowercased before counting.
    #[serde(default)]
    pub preserve_case: bool,
}

fn default_domain_boost() -> u64 {
    10
}

fn default_min_word_length() -> usize {
    2
}

fn default_min_frequency() -> u64 {
    1
}

fn default_max_word_length() -> usize {
    50
}

impl Default for DictionaryConfig {
    fn default() -> Self {
        Self {
            domain_boost: default_domain_boost(),
            min_word_length: default_min_word_length(),
            min_frequency: default_min_frequency(),
            max_word_length: default_max_word_length(),
            preserve_case: false,
        }
    }
}

impl DictionaryConfig {
    /// Create a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the domain boost multiplier.
    pub fn with_domain_boost(mut self, boost: u64) -> Self {
        self.domain_boost = boost;
        self
    }

    /// Set the minimum word length.
    pub fn with_min_word_length(mut self, length: usize) -> Self {
        self.min_word_length = length;
        self
    }

    /// Set the minimum frequency threshold.
    pub fn with_min_frequency(mut self, frequency: u64) -> Self {
        self.min_frequency = frequency;
        self
    }

    /// Set the maximum word length.
    pub fn with_max_word_length(mut self, length: usize) -> Self {
        self.max_word_length = length;
        self
    }

    /// Enable case preservation.
    pub fn with_preserve_case(mut self, preserve: bool) -> Self {
        self.preserve_case = preserve;
        self
    }
}

/// Statistics from dictionary generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryStats {
    /// Number of unique words in the corpus dictionary.
    pub unique_words: usize,

    /// Number of unique bigrams in the bigram dictionary.
    pub unique_bigrams: usize,

    /// Total number of words processed from documents.
    pub total_words_processed: usize,

    /// Number of words filtered out by length constraints.
    pub words_filtered_by_length: usize,

    /// Number of words filtered out by frequency threshold.
    pub words_filtered_by_frequency: usize,
}

/// Generates frequency dictionaries from document content.
///
/// The dictionary generator extracts words from document text, counts frequencies,
/// applies domain boost, and writes SymSpell-compatible dictionary files.
#[derive(Debug, Clone)]
pub struct DictionaryGenerator {
    config: DictionaryConfig,
}

impl DictionaryGenerator {
    /// Create a new dictionary generator with the given configuration.
    pub fn new(config: DictionaryConfig) -> Self {
        Self { config }
    }

    /// Create a dictionary generator with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(DictionaryConfig::default())
    }

    /// Get the current configuration.
    pub fn config(&self) -> &DictionaryConfig {
        &self.config
    }

    /// Build a word frequency dictionary from document texts.
    ///
    /// Extracts words from the provided documents, counts their frequencies,
    /// applies the domain boost multiplier, and writes to the output file.
    ///
    /// # Arguments
    ///
    /// * `documents` - Iterator of document text strings
    /// * `output_path` - Path where the dictionary file will be written
    ///
    /// # Returns
    ///
    /// The number of unique words written to the dictionary.
    ///
    /// # Format
    ///
    /// Output format is SymSpell-compatible: `word frequency\n`
    pub fn build_corpus_dictionary<'a, I>(
        &self,
        documents: I,
        output_path: impl AsRef<Path>,
    ) -> Result<usize>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut word_frequencies: HashMap<String, u64> = HashMap::new();
        let mut total_words = 0usize;
        let mut filtered_by_length = 0usize;

        // Count word frequencies across all documents
        for doc in documents {
            for word in self.tokenize(doc) {
                total_words += 1;

                // Apply length filter
                if word.len() < self.config.min_word_length
                    || word.len() > self.config.max_word_length
                {
                    filtered_by_length += 1;
                    continue;
                }

                // Normalize case if configured
                let normalized = if self.config.preserve_case {
                    word.to_string()
                } else {
                    word.to_lowercase()
                };

                *word_frequencies.entry(normalized).or_insert(0) += 1;
            }
        }

        // Filter by minimum frequency and apply domain boost
        let filtered: HashMap<String, u64> = word_frequencies
            .into_iter()
            .filter(|(_, freq)| *freq >= self.config.min_frequency)
            .map(|(word, freq)| (word, freq * self.config.domain_boost))
            .collect();

        // Ensure parent directory exists
        let output_path = output_path.as_ref();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write dictionary file
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);

        let unique_words = filtered.len();

        // Sort by frequency (descending) for deterministic output
        let mut entries: Vec<_> = filtered.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        for (word, frequency) in entries {
            writeln!(writer, "{} {}", word, frequency)?;
        }

        writer.flush()?;

        tracing::debug!(
            total_words,
            filtered_by_length,
            unique_words,
            "Built corpus dictionary"
        );

        Ok(unique_words)
    }

    /// Build a bigram frequency dictionary for compound word correction.
    ///
    /// Extracts word pairs (bigrams) from consecutive words in documents,
    /// counts their frequencies, and writes to the output file. This enables
    /// the typo corrector to handle compound word errors like "inthe" -> "in the".
    ///
    /// # Arguments
    ///
    /// * `documents` - Iterator of document text strings
    /// * `output_path` - Path where the bigram dictionary file will be written
    ///
    /// # Returns
    ///
    /// The number of unique bigrams written to the dictionary.
    ///
    /// # Format
    ///
    /// Output format is SymSpell-compatible: `word1 word2 frequency\n`
    pub fn build_bigram_dictionary<'a, I>(
        &self,
        documents: I,
        output_path: impl AsRef<Path>,
    ) -> Result<usize>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut bigram_frequencies: HashMap<(String, String), u64> = HashMap::new();

        // Count bigram frequencies across all documents
        for doc in documents {
            let words: Vec<String> = self
                .tokenize(doc)
                .filter(|w| {
                    w.len() >= self.config.min_word_length && w.len() <= self.config.max_word_length
                })
                .map(|w| {
                    if self.config.preserve_case {
                        w.to_string()
                    } else {
                        w.to_lowercase()
                    }
                })
                .collect();

            // Generate bigrams from consecutive words
            for window in words.windows(2) {
                let bigram = (window[0].clone(), window[1].clone());
                *bigram_frequencies.entry(bigram).or_insert(0) += 1;
            }
        }

        // Filter by minimum frequency and apply domain boost
        let filtered: HashMap<(String, String), u64> = bigram_frequencies
            .into_iter()
            .filter(|(_, freq)| *freq >= self.config.min_frequency)
            .map(|(bigram, freq)| (bigram, freq * self.config.domain_boost))
            .collect();

        // Ensure parent directory exists
        let output_path = output_path.as_ref();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write bigram dictionary file
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);

        let unique_bigrams = filtered.len();

        // Sort by frequency (descending) for deterministic output
        let mut entries: Vec<_> = filtered.into_iter().collect();
        entries.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0 .0.cmp(&b.0 .0))
                .then_with(|| a.0 .1.cmp(&b.0 .1))
        });

        for ((word1, word2), frequency) in entries {
            writeln!(writer, "{} {} {}", word1, word2, frequency)?;
        }

        writer.flush()?;

        tracing::debug!(unique_bigrams, "Built bigram dictionary");

        Ok(unique_bigrams)
    }

    /// Rebuild all dictionaries from documents.
    ///
    /// Convenience method that builds both corpus and bigram dictionaries.
    /// The document iterator is cloned three times: once for the corpus
    /// dictionary, once for the bigram dictionary, and once to count total
    /// words for the returned statistics. This is a deliberate trade-off —
    /// each builder has different internal state (word vs. bigram frequencies)
    /// so merging them into a single pass would complicate the code without
    /// meaningful performance gain, since document sets are typically small
    /// (in-memory strings).
    ///
    /// # Arguments
    ///
    /// * `documents` - Iterator of document text strings (must be Clone)
    /// * `corpus_path` - Path for the word frequency dictionary
    /// * `bigram_path` - Path for the bigram dictionary
    ///
    /// # Returns
    ///
    /// Statistics about the generated dictionaries.
    pub fn rebuild_all<'a, I>(
        &self,
        documents: I,
        corpus_path: impl AsRef<Path>,
        bigram_path: impl AsRef<Path>,
    ) -> Result<DictionaryStats>
    where
        I: IntoIterator<Item = &'a str> + Clone,
    {
        // Build corpus dictionary
        let unique_words = self.build_corpus_dictionary(documents.clone(), corpus_path)?;

        // Build bigram dictionary
        let unique_bigrams = self.build_bigram_dictionary(documents.clone(), bigram_path)?;

        // Calculate total words processed
        let total_words_processed: usize = documents.into_iter().map(|d| self.tokenize(d).count()).sum();

        Ok(DictionaryStats {
            unique_words,
            unique_bigrams,
            total_words_processed,
            words_filtered_by_length: 0, // Not tracked in rebuild_all
            words_filtered_by_frequency: 0, // Not tracked in rebuild_all
        })
    }

    /// Tokenize document text into words.
    ///
    /// Splits on whitespace and strips punctuation from word boundaries.
    /// Returns an iterator of word slices.
    fn tokenize<'a>(&self, text: &'a str) -> impl Iterator<Item = &'a str> {
        text.split_whitespace()
            .map(|word| strip_punctuation(word))
            .filter(|word| !word.is_empty())
    }

    /// Merge multiple dictionary files into one.
    ///
    /// Useful for combining a domain-specific corpus dictionary with a
    /// general language dictionary. Frequencies from all sources are summed.
    ///
    /// # Arguments
    ///
    /// * `input_paths` - Paths to dictionary files to merge
    /// * `output_path` - Path for the merged dictionary
    ///
    /// # Returns
    ///
    /// The number of unique words in the merged dictionary.
    pub fn merge_dictionaries<P, I>(input_paths: I, output_path: impl AsRef<Path>) -> Result<usize>
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = P>,
    {
        let mut merged: HashMap<String, u64> = HashMap::new();

        for path in input_paths {
            let content = fs::read_to_string(path.as_ref())?;

            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let word = parts[0].to_string();
                    if let Ok(freq) = parts[1].parse::<u64>() {
                        *merged.entry(word).or_insert(0) += freq;
                    }
                }
            }
        }

        // Ensure parent directory exists
        let output_path = output_path.as_ref();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write merged dictionary
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);

        let unique_words = merged.len();

        // Sort by frequency (descending)
        let mut entries: Vec<_> = merged.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        for (word, frequency) in entries {
            writeln!(writer, "{} {}", word, frequency)?;
        }

        writer.flush()?;

        Ok(unique_words)
    }
}

impl Default for DictionaryGenerator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Strip leading and trailing punctuation from a word.
///
/// Preserves internal punctuation (e.g., hyphenated words, apostrophes).
fn strip_punctuation(word: &str) -> &str {
    let start = word
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(word.len());
    let end = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    if start >= end {
        return "";
    }

    &word[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = DictionaryConfig::default();
        assert_eq!(config.domain_boost, 10);
        assert_eq!(config.min_word_length, 2);
        assert_eq!(config.min_frequency, 1);
        assert_eq!(config.max_word_length, 50);
        assert!(!config.preserve_case);
    }

    #[test]
    fn test_config_builder() {
        let config = DictionaryConfig::new()
            .with_domain_boost(5)
            .with_min_word_length(3)
            .with_min_frequency(2)
            .with_max_word_length(30)
            .with_preserve_case(true);

        assert_eq!(config.domain_boost, 5);
        assert_eq!(config.min_word_length, 3);
        assert_eq!(config.min_frequency, 2);
        assert_eq!(config.max_word_length, 30);
        assert!(config.preserve_case);
    }

    #[test]
    fn test_strip_punctuation() {
        assert_eq!(strip_punctuation("hello"), "hello");
        assert_eq!(strip_punctuation("(hello)"), "hello");
        assert_eq!(strip_punctuation("hello,"), "hello");
        assert_eq!(strip_punctuation("...test!!!"), "test");
        assert_eq!(strip_punctuation("don't"), "don't");
        assert_eq!(strip_punctuation("well-known"), "well-known");
        assert_eq!(strip_punctuation("..."), "");
    }

    #[test]
    fn test_strip_punctuation_multibyte() {
        assert_eq!(strip_punctuation("café"), "café");
        assert_eq!(strip_punctuation("¡café!"), "café");
        assert_eq!(strip_punctuation("über"), "über");
        assert_eq!(strip_punctuation("日本語"), "日本語");
        assert_eq!(strip_punctuation("«résumé»"), "résumé");
        assert_eq!(strip_punctuation("naïve"), "naïve");
    }

    #[test]
    fn test_build_corpus_dictionary() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec![
            "search engine is fast",
            "search is powerful",
            "engine runs well",
        ];

        let generator = DictionaryGenerator::with_defaults();
        let unique_words = generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        assert!(unique_words > 0);
        assert!(dict_path.exists());

        // Read and verify content
        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // "search" appears twice, so with domain boost of 10, frequency should be 20
        assert!(content.contains("search 20"));
        // "engine" appears twice
        assert!(content.contains("engine 20"));
        // "is" appears twice
        assert!(content.contains("is 20"));
        // "fast" appears once
        assert!(content.contains("fast 10"));
    }

    #[test]
    fn test_build_bigram_dictionary() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("bigram.txt");

        let documents = vec![
            "search engine rocks",
            "search engine is fast",
            "fast search wins",
        ];

        let generator = DictionaryGenerator::with_defaults();
        let unique_bigrams = generator
            .build_bigram_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        assert!(unique_bigrams > 0);
        assert!(dict_path.exists());

        // Read and verify content
        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // "search engine" appears twice
        assert!(content.contains("search engine 20"));
    }

    #[test]
    fn test_min_word_length_filter() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec!["a is in the box"];

        // With default min_word_length of 2, single-char words should be excluded
        let generator = DictionaryGenerator::with_defaults();
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // "a" should be filtered out
        assert!(!content.contains("a "));
        // "is", "in", "the", "box" should be included
        assert!(content.contains("is "));
        assert!(content.contains("in "));
        assert!(content.contains("the "));
        assert!(content.contains("box "));
    }

    #[test]
    fn test_min_frequency_filter() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec![
            "search search search",
            "rare word here",
        ];

        // Require minimum frequency of 2
        let config = DictionaryConfig::new().with_min_frequency(2);
        let generator = DictionaryGenerator::new(config);
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // "search" appears 3 times, should be included
        assert!(content.contains("search"));
        // "rare", "word", "here" appear only once, should be excluded
        assert!(!content.contains("rare"));
        assert!(!content.contains("here"));
    }

    #[test]
    fn test_case_normalization() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec!["Search SEARCH search"];

        // Without preserve_case (default), all should be merged
        let generator = DictionaryGenerator::with_defaults();
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // All variations should be merged into lowercase "search" with frequency 30
        assert!(content.contains("search 30"));
        assert!(!content.contains("Search"));
        assert!(!content.contains("SEARCH"));
    }

    #[test]
    fn test_preserve_case() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec!["Search SEARCH search"];

        // With preserve_case, each variation should be separate
        let config = DictionaryConfig::new().with_preserve_case(true);
        let generator = DictionaryGenerator::new(config);
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // Each case variation should be separate
        assert!(content.contains("Search 10"));
        assert!(content.contains("SEARCH 10"));
        assert!(content.contains("search 10"));
    }

    #[test]
    fn test_rebuild_all() {
        let temp_dir = TempDir::new().unwrap();
        let corpus_path = temp_dir.path().join("corpus.txt");
        let bigram_path = temp_dir.path().join("bigram.txt");

        let documents = vec![
            "search engine is great",
            "search engine rocks",
        ];

        let generator = DictionaryGenerator::with_defaults();
        let stats = generator
            .rebuild_all(
                documents.iter().map(|s| *s),
                &corpus_path,
                &bigram_path,
            )
            .unwrap();

        assert!(stats.unique_words > 0);
        assert!(stats.unique_bigrams > 0);
        assert!(stats.total_words_processed > 0);
        assert!(corpus_path.exists());
        assert!(bigram_path.exists());
    }

    #[test]
    fn test_merge_dictionaries() {
        let temp_dir = TempDir::new().unwrap();
        let dict1_path = temp_dir.path().join("dict1.txt");
        let dict2_path = temp_dir.path().join("dict2.txt");
        let merged_path = temp_dir.path().join("merged.txt");

        // Create first dictionary
        fs::write(&dict1_path, "search 100\nengine 50\n").unwrap();

        // Create second dictionary
        fs::write(&dict2_path, "search 50\nfast 30\n").unwrap();

        let unique_words =
            DictionaryGenerator::merge_dictionaries([&dict1_path, &dict2_path], &merged_path)
                .unwrap();

        assert_eq!(unique_words, 3); // search, engine, fast

        let mut content = String::new();
        File::open(&merged_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // "search" should have combined frequency of 150
        assert!(content.contains("search 150"));
        // "engine" should have frequency of 50
        assert!(content.contains("engine 50"));
        // "fast" should have frequency of 30
        assert!(content.contains("fast 30"));
    }

    #[test]
    fn test_punctuation_handling() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec![
            "Hello, world! How are you?",
            "(parenthetical) and \"quoted\" text.",
        ];

        let generator = DictionaryGenerator::with_defaults();
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let mut content = String::new();
        File::open(&dict_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        // Words should have punctuation stripped
        assert!(content.contains("hello"));
        assert!(content.contains("world"));
        assert!(content.contains("parenthetical"));
        assert!(content.contains("quoted"));

        // Punctuation should not appear as standalone entries
        let lines: Vec<&str> = content.lines().collect();
        for line in lines {
            let word = line.split_whitespace().next().unwrap_or("");
            assert!(
                word.chars().all(|c| c.is_alphanumeric() || c == '\'' || c == '-'),
                "Unexpected punctuation in word: {}",
                word
            );
        }
    }

    #[test]
    fn test_empty_documents() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents: Vec<&str> = vec![];

        let generator = DictionaryGenerator::with_defaults();
        let unique_words = generator
            .build_corpus_dictionary(documents.into_iter(), &dict_path)
            .unwrap();

        assert_eq!(unique_words, 0);
        assert!(dict_path.exists());

        let content = fs::read_to_string(&dict_path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("a/b/c/corpus.txt");

        let documents = vec!["test document"];

        let generator = DictionaryGenerator::with_defaults();
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &nested_path)
            .unwrap();

        assert!(nested_path.exists());
    }

    #[test]
    fn test_domain_boost_application() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec!["test"];

        // Use domain boost of 5
        let config = DictionaryConfig::new().with_domain_boost(5);
        let generator = DictionaryGenerator::new(config);
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let content = fs::read_to_string(&dict_path).unwrap();
        // "test" appears once, with boost of 5, frequency should be 5
        assert!(content.contains("test 5"));
    }

    #[test]
    fn test_max_word_length_filter() {
        let temp_dir = TempDir::new().unwrap();
        let dict_path = temp_dir.path().join("corpus.txt");

        let documents = vec!["short verylongwordthatexceedsmaxlength"];

        let config = DictionaryConfig::new().with_max_word_length(10);
        let generator = DictionaryGenerator::new(config);
        generator
            .build_corpus_dictionary(documents.iter().map(|s| *s), &dict_path)
            .unwrap();

        let content = fs::read_to_string(&dict_path).unwrap();
        assert!(content.contains("short"));
        assert!(!content.contains("verylongwordthatexceedsmaxlength"));
    }
}
