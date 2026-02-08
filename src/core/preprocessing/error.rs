//! Error types for the preprocessing module.

use thiserror::Error;

/// Errors that can occur during query preprocessing.
#[derive(Debug, Error)]
pub enum PreprocessingError {
    /// Dictionary file not found.
    #[error("Dictionary file not found: {0}")]
    DictionaryNotFound(String),

    /// Failed to load a dictionary.
    #[error("Dictionary load failed: {0}")]
    DictionaryLoad(String),

    /// Configuration load failed (file I/O).
    #[error("Configuration load failed: {0}")]
    ConfigLoad(String),

    /// Configuration parse failed.
    #[error("Configuration parse failed: {0}")]
    ConfigParse(String),

    /// Invalid configuration value.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// I/O error during file operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON parsing error.
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML parsing error.
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Result type for preprocessing operations.
pub type Result<T> = std::result::Result<T, PreprocessingError>;
