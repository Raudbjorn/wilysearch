//! Default synonym definitions for common domains.
//!
//! Each sub-module provides a `build_default_*_synonyms()` function that
//! returns a pre-populated [`SynonymMap`](super::SynonymMap).

pub mod ttrpg;

pub use ttrpg::build_default_ttrpg_synonyms;
