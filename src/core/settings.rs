//! Settings shared with the public API and the pinned upstream engine.
pub use crate::types::Settings;
pub use crate::types::{
    FacetValuesSort, Faceting as FacetingSettings, LocalizedAttribute as LocalizedAttributeRule,
    MinWordSizeForTypos, Pagination as PaginationSettings, ProximityPrecision,
    TypoTolerance as TypoToleranceSettings,
};
pub use milli::update::Setting;
pub use milli::vector::settings::{EmbedderSource, EmbeddingSettings as EmbedderSettings};
