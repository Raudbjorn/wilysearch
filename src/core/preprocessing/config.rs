//! Configuration loading for preprocessing components.
//!
//! This module provides configuration structures for the preprocessing pipeline,
//! including typo correction, synonym expansion, dictionary paths, and normalization.
//!
//! # Example Configuration File (TOML)
//!
//! ```toml
//! [typo]
//! min_word_size_one_typo = 5
//! min_word_size_two_typos = 9
//! max_edit_distance = 2
//! enabled = true
//! disabled_on_words = ["dnd", "5e", "phb"]
//!
//! [synonyms]
//! max_expansions = 5
//! enabled = true
//! include_original = true
//!
//! [paths]
//! english_dict = "data/frequency_dictionary_en.txt"
//! corpus_dict = "data/corpus.txt"
//!
//! [normalization]
//! lowercase = true
//! trim = true
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::error::{PreprocessingError, Result};
use super::synonyms::SynonymConfig;
use super::typo::TypoConfig;

/// Complete preprocessing configuration.
///
/// This structure encompasses all configuration options for the preprocessing
/// pipeline, including typo correction, synonym expansion, dictionary paths,
/// and text normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PreprocessingConfig {
    /// Typo correction settings.
    pub typo: TypoConfig,

    /// Synonym expansion settings.
    pub synonyms: SynonymConfig,

    /// Paths to dictionary files.
    pub paths: DictionaryPaths,

    /// Normalization settings.
    pub normalization: NormalizationConfig,
}

/// Paths to dictionary files used by the preprocessing pipeline.
///
/// All paths are optional; if not specified, the corresponding feature
/// will not be loaded from file (but can still be populated programmatically).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionaryPaths {
    /// Path to English frequency dictionary.
    ///
    /// Format: `word frequency` (space-separated, one per line)
    /// Example:
    /// ```text
    /// the 23135851162
    /// of 13151942776
    /// ```
    pub english_dict: Option<PathBuf>,

    /// Path to corpus frequency dictionary (generated from domain content).
    ///
    /// Same format as `english_dict`.
    pub corpus_dict: Option<PathBuf>,

    /// Path to bigram dictionary (for compound word correction).
    ///
    /// Format: `word1 word2 frequency` (space-separated, one per line)
    /// Example:
    /// ```text
    /// in the 123456
    /// of the 98765
    /// ```
    pub bigram_dict: Option<PathBuf>,

    /// Path to synonyms TOML file.
    ///
    /// The file should follow the synonym configuration format.
    pub synonyms_file: Option<PathBuf>,
}

/// Normalization settings for query preprocessing.
///
/// These settings control how text is normalized before typo correction
/// and synonym expansion are applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalizationConfig {
    /// Convert to lowercase.
    ///
    /// When enabled, all text is converted to lowercase before processing.
    /// This ensures case-insensitive matching in dictionaries.
    pub lowercase: bool,

    /// Trim leading and trailing whitespace.
    pub trim: bool,

    /// Collapse multiple consecutive whitespace characters into a single space.
    pub collapse_whitespace: bool,

    /// Apply Unicode normalization (NFKC).
    ///
    /// NFKC normalization converts compatibility characters to their canonical
    /// equivalents (e.g., full-width characters, ligatures).
    pub unicode_normalize: bool,
}

impl Default for PreprocessingConfig {
    fn default() -> Self {
        Self {
            typo: TypoConfig::default(),
            synonyms: SynonymConfig::default(),
            paths: DictionaryPaths::default(),
            normalization: NormalizationConfig::default(),
        }
    }
}

impl Default for DictionaryPaths {
    fn default() -> Self {
        Self {
            english_dict: None,
            corpus_dict: None,
            bigram_dict: None,
            synonyms_file: None,
        }
    }
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            lowercase: true,
            trim: true,
            collapse_whitespace: true,
            unicode_normalize: false,
        }
    }
}

impl PreprocessingConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Arguments
    /// * `path` - Path to the TOML configuration file
    ///
    /// # Example
    /// ```ignore
    /// let config = PreprocessingConfig::from_toml("config/preprocessing.toml")?;
    /// ```
    pub fn from_toml(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| PreprocessingError::ConfigLoad(e.to_string()))?;

        toml::from_str(&content).map_err(|e| PreprocessingError::ConfigParse(e.to_string()))
    }

    /// Load configuration from a TOML string.
    ///
    /// # Arguments
    /// * `content` - TOML configuration content as a string
    pub fn from_toml_str(content: &str) -> Result<Self> {
        toml::from_str(content).map_err(|e| PreprocessingError::ConfigParse(e.to_string()))
    }

    /// Save configuration to a TOML file.
    ///
    /// # Arguments
    /// * `path` - Path where the configuration file will be written
    pub fn to_toml(&self, path: impl AsRef<Path>) -> Result<()> {
        let content =
            toml::to_string_pretty(self).map_err(|e| PreprocessingError::ConfigParse(e.to_string()))?;

        std::fs::write(path.as_ref(), content)
            .map_err(|e| PreprocessingError::ConfigLoad(e.to_string()))
    }

    /// Serialize configuration to a TOML string.
    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| PreprocessingError::ConfigParse(e.to_string()))
    }

    /// Create a default configuration file at the specified path.
    ///
    /// This is useful for generating template configuration files.
    pub fn write_default(path: impl AsRef<Path>) -> Result<()> {
        Self::default().to_toml(path)
    }

    /// Validate the configuration.
    ///
    /// Checks that:
    /// - Typo tolerance thresholds are in valid order
    /// - Dictionary paths (if specified) exist
    ///
    /// # Arguments
    /// * `check_paths` - Whether to verify that configured paths exist on disk
    pub fn validate(&self, check_paths: bool) -> Result<()> {
        // Validate typo config
        if self.typo.min_word_size_one_typo > self.typo.min_word_size_two_typos {
            return Err(PreprocessingError::InvalidConfig(
                "min_word_size_one_typo must be <= min_word_size_two_typos".to_string(),
            ));
        }

        if self.typo.max_edit_distance < 0 || self.typo.max_edit_distance > 3 {
            return Err(PreprocessingError::InvalidConfig(
                "max_edit_distance must be between 0 and 3".to_string(),
            ));
        }

        // Validate paths if requested
        if check_paths {
            if let Some(ref path) = self.paths.english_dict {
                if !path.exists() {
                    return Err(PreprocessingError::DictionaryNotFound(
                        path.display().to_string(),
                    ));
                }
            }

            if let Some(ref path) = self.paths.corpus_dict {
                if !path.exists() {
                    return Err(PreprocessingError::DictionaryNotFound(
                        path.display().to_string(),
                    ));
                }
            }

            if let Some(ref path) = self.paths.bigram_dict {
                if !path.exists() {
                    return Err(PreprocessingError::DictionaryNotFound(
                        path.display().to_string(),
                    ));
                }
            }

            if let Some(ref path) = self.paths.synonyms_file {
                if !path.exists() {
                    return Err(PreprocessingError::ConfigLoad(format!(
                        "Synonyms file not found: {}",
                        path.display()
                    )));
                }
            }
        }

        Ok(())
    }

    /// Create a builder for constructing a PreprocessingConfig.
    pub fn builder() -> PreprocessingConfigBuilder {
        PreprocessingConfigBuilder::default()
    }
}

impl DictionaryPaths {
    /// Create a new DictionaryPaths with all paths set.
    pub fn new(
        english_dict: impl Into<Option<PathBuf>>,
        corpus_dict: impl Into<Option<PathBuf>>,
        bigram_dict: impl Into<Option<PathBuf>>,
        synonyms_file: impl Into<Option<PathBuf>>,
    ) -> Self {
        Self {
            english_dict: english_dict.into(),
            corpus_dict: corpus_dict.into(),
            bigram_dict: bigram_dict.into(),
            synonyms_file: synonyms_file.into(),
        }
    }

    /// Check if any dictionary paths are configured.
    pub fn has_any(&self) -> bool {
        self.english_dict.is_some()
            || self.corpus_dict.is_some()
            || self.bigram_dict.is_some()
            || self.synonyms_file.is_some()
    }

    /// Get all configured paths as a vector.
    pub fn all_paths(&self) -> Vec<&Path> {
        let mut paths = Vec::new();
        if let Some(ref p) = self.english_dict {
            paths.push(p.as_path());
        }
        if let Some(ref p) = self.corpus_dict {
            paths.push(p.as_path());
        }
        if let Some(ref p) = self.bigram_dict {
            paths.push(p.as_path());
        }
        if let Some(ref p) = self.synonyms_file {
            paths.push(p.as_path());
        }
        paths
    }
}

impl NormalizationConfig {
    /// Create a configuration with all normalization options enabled.
    pub fn all_enabled() -> Self {
        Self {
            lowercase: true,
            trim: true,
            collapse_whitespace: true,
            unicode_normalize: true,
        }
    }

    /// Create a configuration with all normalization options disabled.
    pub fn none() -> Self {
        Self {
            lowercase: false,
            trim: false,
            collapse_whitespace: false,
            unicode_normalize: false,
        }
    }
}

/// Builder for constructing a PreprocessingConfig.
#[derive(Debug, Default)]
pub struct PreprocessingConfigBuilder {
    config: PreprocessingConfig,
}

impl PreprocessingConfigBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the typo configuration.
    pub fn typo(mut self, config: TypoConfig) -> Self {
        self.config.typo = config;
        self
    }

    /// Set the synonym configuration.
    pub fn synonyms(mut self, config: SynonymConfig) -> Self {
        self.config.synonyms = config;
        self
    }

    /// Set the dictionary paths.
    pub fn paths(mut self, paths: DictionaryPaths) -> Self {
        self.config.paths = paths;
        self
    }

    /// Set the normalization configuration.
    pub fn normalization(mut self, config: NormalizationConfig) -> Self {
        self.config.normalization = config;
        self
    }

    /// Set the English dictionary path.
    pub fn english_dict(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.paths.english_dict = Some(path.into());
        self
    }

    /// Set the corpus dictionary path.
    pub fn corpus_dict(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.paths.corpus_dict = Some(path.into());
        self
    }

    /// Set the bigram dictionary path.
    pub fn bigram_dict(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.paths.bigram_dict = Some(path.into());
        self
    }

    /// Set the synonyms file path.
    pub fn synonyms_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.paths.synonyms_file = Some(path.into());
        self
    }

    /// Enable/disable lowercase normalization.
    pub fn lowercase(mut self, enabled: bool) -> Self {
        self.config.normalization.lowercase = enabled;
        self
    }

    /// Enable/disable whitespace trimming.
    pub fn trim(mut self, enabled: bool) -> Self {
        self.config.normalization.trim = enabled;
        self
    }

    /// Enable/disable typo correction.
    pub fn typo_enabled(mut self, enabled: bool) -> Self {
        self.config.typo.enabled = enabled;
        self
    }

    /// Enable/disable synonym expansion.
    pub fn synonyms_enabled(mut self, enabled: bool) -> Self {
        self.config.synonyms.enabled = enabled;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> PreprocessingConfig {
        self.config
    }

    /// Build and validate the configuration.
    ///
    /// # Arguments
    /// * `check_paths` - Whether to verify that configured paths exist on disk
    pub fn build_validated(self, check_paths: bool) -> Result<PreprocessingConfig> {
        let config = self.build();
        config.validate(check_paths)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PreprocessingConfig::default();
        assert!(config.typo.enabled);
        assert!(config.synonyms.enabled);
        assert!(config.normalization.lowercase);
        assert!(config.normalization.trim);
        assert!(config.paths.english_dict.is_none());
    }

    #[test]
    fn test_config_from_toml() {
        // Note: TypoConfig and SynonymConfig use camelCase serde rename
        let toml = r#"
            [typo]
            minWordSizeOneTypo = 4
            minWordSizeTwoTypos = 8
            maxEditDistance = 2
            enabled = true
            disabledOnWords = ["api", "http"]

            [synonyms]
            maxExpansions = 5
            enabled = true
            includeOriginal = true

            [paths]
            english_dict = "/data/english.txt"
            corpus_dict = "/data/corpus.txt"

            [normalization]
            lowercase = true
            trim = true
            collapse_whitespace = true
            unicode_normalize = false
        "#;

        let config = PreprocessingConfig::from_toml_str(toml).unwrap();
        assert_eq!(config.typo.min_word_size_one_typo, 4);
        assert_eq!(config.typo.min_word_size_two_typos, 8);
        assert_eq!(config.synonyms.max_expansions, 5);
        assert_eq!(
            config.paths.english_dict,
            Some(PathBuf::from("/data/english.txt"))
        );
        assert!(config.normalization.lowercase);
    }

    #[test]
    fn test_config_to_toml_string() {
        let config = PreprocessingConfig::default();
        let toml = config.to_toml_string().unwrap();
        assert!(toml.contains("[typo]"));
        assert!(toml.contains("[synonyms]"));
        assert!(toml.contains("[normalization]"));
    }

    #[test]
    fn test_config_validation() {
        let mut config = PreprocessingConfig::default();
        config.typo.min_word_size_one_typo = 10;
        config.typo.min_word_size_two_typos = 5;

        let result = config.validate(false);
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_pattern() {
        let config = PreprocessingConfig::builder()
            .typo_enabled(false)
            .synonyms_enabled(true)
            .lowercase(true)
            .english_dict("/path/to/dict.txt")
            .build();

        assert!(!config.typo.enabled);
        assert!(config.synonyms.enabled);
        assert!(config.normalization.lowercase);
        assert_eq!(
            config.paths.english_dict,
            Some(PathBuf::from("/path/to/dict.txt"))
        );
    }

    #[test]
    fn test_dictionary_paths_has_any() {
        let empty = DictionaryPaths::default();
        assert!(!empty.has_any());

        let with_path = DictionaryPaths {
            english_dict: Some(PathBuf::from("/test")),
            ..Default::default()
        };
        assert!(with_path.has_any());
    }

    #[test]
    fn test_normalization_presets() {
        let all = NormalizationConfig::all_enabled();
        assert!(all.lowercase);
        assert!(all.trim);
        assert!(all.collapse_whitespace);
        assert!(all.unicode_normalize);

        let none = NormalizationConfig::none();
        assert!(!none.lowercase);
        assert!(!none.trim);
        assert!(!none.collapse_whitespace);
        assert!(!none.unicode_normalize);
    }
}
