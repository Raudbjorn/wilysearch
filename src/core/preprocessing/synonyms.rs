//! Synonym expansion module for query preprocessing.
//!
//! This module provides bidirectional synonym mapping with support for:
//! - Multi-way synonyms (all terms interchangeable)
//! - One-way synonyms (source maps to targets, not vice versa)
//! - Query expansion with configurable limits
//! - Database-specific query generation (SurrealDB FTS, SQLite FTS5)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::core::preprocessing::error::{PreprocessingError, Result};

/// Type of synonym relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynonymType {
    /// All terms are interchangeable. If A, B, C are multi-way synonyms,
    /// searching for A returns results for A, B, and C.
    MultiWay,
    /// Source term maps to target terms, but not vice versa.
    /// If "dragon" -> ["wyrm", "drake"], searching "dragon" finds "wyrm" and "drake",
    /// but searching "wyrm" does NOT find "dragon".
    OneWay,
}

/// A single term in an expanded query, with its expansion source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedTerm {
    /// The term text.
    pub term: String,
    /// Whether this is the original term or an expansion.
    pub is_original: bool,
    /// The original term this was expanded from, if applicable.
    pub expanded_from: Option<String>,
}

impl ExpandedTerm {
    /// Create a new expanded term representing the original.
    pub fn original(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            is_original: true,
            expanded_from: None,
        }
    }

    /// Create a new expanded term representing a synonym expansion.
    pub fn expansion(term: impl Into<String>, from: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            is_original: false,
            expanded_from: Some(from.into()),
        }
    }
}

/// A group of alternative terms representing a single query position.
///
/// For example, if the user searches "hp recovery" and "hp" has synonyms
/// ["hit points", "health"], this would be represented as:
/// - Position 0: ["hp", "hit points", "health"]
/// - Position 1: ["recovery"]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TermAlternatives {
    /// All alternative terms for this position.
    pub alternatives: Vec<ExpandedTerm>,
}

impl TermAlternatives {
    /// Create a new term alternatives group with a single original term.
    pub fn new(original: impl Into<String>) -> Self {
        Self {
            alternatives: vec![ExpandedTerm::original(original)],
        }
    }

    /// Add an expansion to this group.
    pub fn add_expansion(&mut self, term: impl Into<String>, from: impl Into<String>) {
        self.alternatives.push(ExpandedTerm::expansion(term, from));
    }

    /// Get the original term.
    pub fn original(&self) -> Option<&str> {
        self.alternatives
            .iter()
            .find(|t| t.is_original)
            .map(|t| t.term.as_str())
    }

    /// Get all term strings.
    pub fn all_terms(&self) -> Vec<&str> {
        self.alternatives.iter().map(|t| t.term.as_str()).collect()
    }

    /// Check if this group has any expansions.
    pub fn has_expansions(&self) -> bool {
        self.alternatives.iter().any(|t| !t.is_original)
    }
}

/// An expanded query with synonym alternatives at each position.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExpandedQuery {
    /// The original query string.
    pub original_query: String,
    /// Term groups with alternatives at each position.
    pub term_groups: Vec<TermAlternatives>,
    /// Whether any expansions were applied.
    pub has_expansions: bool,
}

impl ExpandedQuery {
    /// Create a new expanded query.
    pub fn new(original_query: impl Into<String>) -> Self {
        Self {
            original_query: original_query.into(),
            term_groups: Vec::new(),
            has_expansions: false,
        }
    }

    /// Add a term group (position) to the query.
    pub fn add_group(&mut self, group: TermAlternatives) {
        if group.has_expansions() {
            self.has_expansions = true;
        }
        self.term_groups.push(group);
    }

    /// Get the query as a simple string (original terms only).
    pub fn to_simple_string(&self) -> String {
        self.term_groups
            .iter()
            .filter_map(|g| g.original())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Generate a SurrealDB FTS (full-text search) query.
    ///
    /// SurrealDB uses the `@@` operator for FTS matching and supports OR groups.
    /// Output format: `field @@ 'term1 OR term2' AND field @@ 'term3'`
    ///
    /// # Arguments
    /// * `field` - The field name to search (e.g., "content")
    /// * `ref_num` - A reference number for query parameterization
    ///
    /// # Example
    /// For query "hp recovery" with "hp" -> ["hit points", "health"]:
    /// ```text
    /// content @@ '(hp OR "hit points" OR health)' AND content @@ 'recovery'
    /// ```
    pub fn to_surrealdb_fts(&self, field: &str, _ref_num: u32) -> String {
        let parts: Vec<String> = self
            .term_groups
            .iter()
            .map(|group| {
                let terms = group.all_terms();
                if terms.len() == 1 {
                    format!("{} @@ '{}'", field, escape_fts_term(&terms[0]))
                } else {
                    let or_terms: Vec<String> = terms
                        .iter()
                        .map(|t| {
                            if t.contains(' ') {
                                format!("\"{}\"", escape_fts_term(t))
                            } else {
                                escape_fts_term(t)
                            }
                        })
                        .collect();
                    format!("{} @@ '({})'", field, or_terms.join(" OR "))
                }
            })
            .collect();

        parts.join(" AND ")
    }

    /// Generate an SQLite FTS5 MATCH expression.
    ///
    /// FTS5 uses boolean operators within the MATCH string.
    /// Output format: `(term1 OR term2) AND term3`
    ///
    /// # Example
    /// For query "hp recovery" with "hp" -> ["hit points", "health"]:
    /// ```text
    /// (hp OR "hit points" OR health) AND recovery
    /// ```
    pub fn to_fts5_match(&self) -> String {
        let parts: Vec<String> = self
            .term_groups
            .iter()
            .map(|group| {
                let terms = group.all_terms();
                if terms.len() == 1 {
                    escape_fts5_term(&terms[0])
                } else {
                    let or_terms: Vec<String> = terms
                        .iter()
                        .map(|t| {
                            if t.contains(' ') {
                                format!("\"{}\"", escape_fts5_term(t))
                            } else {
                                escape_fts5_term(t)
                            }
                        })
                        .collect();
                    format!("({})", or_terms.join(" OR "))
                }
            })
            .collect();

        parts.join(" AND ")
    }

    /// Generate an expanded query string with all alternatives.
    ///
    /// This produces a human-readable string showing all expansions.
    /// Useful for debugging or displaying to users.
    pub fn to_expanded_string(&self) -> String {
        self.term_groups
            .iter()
            .map(|group| {
                let terms = group.all_terms();
                if terms.len() == 1 {
                    terms[0].to_string()
                } else {
                    format!("[{}]", terms.join("|"))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Escape a term for SurrealDB FTS.
fn escape_fts_term(term: &str) -> String {
    // Escape single quotes by doubling them
    term.replace('\'', "''")
}

/// Escape a term for SQLite FTS5.
fn escape_fts5_term(term: &str) -> String {
    // FTS5 special characters: " * ^ - :
    // FTS5 reserved keywords that would be interpreted as operators if unquoted:
    // AND, OR, NOT, NEAR (AND/OR are used intentionally in our MATCH expressions,
    // but NOT and NEAR would corrupt the query if a synonym term is literally
    // "not" or "near").
    let upper = term.to_uppercase();
    let is_reserved_keyword = matches!(upper.as_str(), "NOT" | "NEAR");

    if is_reserved_keyword
        || term
            .chars()
            .any(|c| matches!(c, '"' | '*' | '^' | '-' | ':'))
    {
        // Escape double quotes by doubling them, then wrap in quotes
        format!("\"{}\"", term.replace('"', "\"\""))
    } else {
        term.to_string()
    }
}

/// Configuration for synonym expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynonymConfig {
    /// Maximum number of expansions per term to prevent query explosion.
    /// Default: 10
    #[serde(default = "default_max_expansions")]
    pub max_expansions: usize,

    /// Whether synonym expansion is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Whether to include the original term in expansions.
    /// Default: true
    #[serde(default = "default_include_original")]
    pub include_original: bool,
}

fn default_max_expansions() -> usize {
    10
}

fn default_enabled() -> bool {
    true
}

fn default_include_original() -> bool {
    true
}

impl Default for SynonymConfig {
    fn default() -> Self {
        Self {
            max_expansions: default_max_expansions(),
            enabled: default_enabled(),
            include_original: default_include_original(),
        }
    }
}

/// Bidirectional synonym map for query expansion.
///
/// Supports both multi-way synonyms (all terms interchangeable) and
/// one-way synonyms (source maps to targets only).
#[derive(Debug, Clone, Default, Serialize)]
pub struct SynonymMap {
    /// Multi-way synonym groups. Each group is a set of interchangeable terms.
    /// Stored as: term -> group_id, where all terms in a group share the same id.
    #[serde(skip)]
    multi_way_index: HashMap<String, usize>,

    /// The actual multi-way synonym groups.
    multi_way_groups: Vec<HashSet<String>>,

    /// One-way synonyms: source -> targets.
    one_way_mappings: HashMap<String, Vec<String>>,

    /// Configuration for expansion behavior.
    #[serde(default)]
    pub config: SynonymConfig,
}

impl<'de> Deserialize<'de> for SynonymMap {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SynonymMapHelper {
            #[serde(default)]
            multi_way_groups: Vec<HashSet<String>>,
            #[serde(default)]
            one_way_mappings: HashMap<String, Vec<String>>,
            #[serde(default)]
            config: SynonymConfig,
        }

        let helper = SynonymMapHelper::deserialize(deserializer)?;

        // Rebuild the multi_way_index from multi_way_groups
        let mut multi_way_index = HashMap::new();
        for (group_id, group) in helper.multi_way_groups.iter().enumerate() {
            for term in group {
                multi_way_index.insert(term.clone(), group_id);
            }
        }

        Ok(SynonymMap {
            multi_way_index,
            multi_way_groups: helper.multi_way_groups,
            one_way_mappings: helper.one_way_mappings,
            config: helper.config,
        })
    }
}

impl SynonymMap {
    /// Create a new empty synonym map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a synonym map with custom configuration.
    pub fn with_config(config: SynonymConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Add a multi-way synonym group where all terms are interchangeable.
    ///
    /// # Example
    /// ```ignore
    /// map.add_multi_way(&["hp", "hit points", "health"]);
    /// // Now searching for any of these terms will match all of them.
    /// ```
    pub fn add_multi_way<I, S>(&mut self, terms: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let terms: HashSet<String> = terms
            .into_iter()
            .map(|s| s.as_ref().to_lowercase())
            .collect();

        if terms.len() < 2 {
            return; // Need at least 2 terms for a synonym group
        }

        let mut affected_group_ids: Vec<usize> = terms
            .iter()
            .filter_map(|t| self.multi_way_index.get(t).copied())
            .collect();
        affected_group_ids.sort_unstable();
        affected_group_ids.dedup();

        match affected_group_ids.len() {
            0 => {
                let group_id = self.multi_way_groups.len();
                for term in &terms {
                    self.multi_way_index.insert(term.clone(), group_id);
                }
                self.multi_way_groups.push(terms);
            }
            1 => {
                let group_id = affected_group_ids[0];
                for term in &terms {
                    self.multi_way_index.insert(term.clone(), group_id);
                }
                self.multi_way_groups[group_id].extend(terms);
            }
            _ => {
                let target_id = affected_group_ids[0];
                let mut merged = std::mem::take(&mut self.multi_way_groups[target_id]);
                merged.extend(terms);

                for &other_id in affected_group_ids[1..].iter().rev() {
                    let other = std::mem::take(&mut self.multi_way_groups[other_id]);
                    // Note: std::mem::take leaves an empty HashSet at other_id.
                    // We clean these up below.
                    merged.extend(other);
                }

                for term in &merged {
                    self.multi_way_index.insert(term.clone(), target_id);
                }
                self.multi_way_groups[target_id] = merged;

                // Remove empty groups left behind by std::mem::take and
                // re-index so group IDs remain contiguous.
                self.multi_way_groups.retain(|g| !g.is_empty());
                self.multi_way_index.clear();
                for (idx, group) in self.multi_way_groups.iter().enumerate() {
                    for term in group {
                        self.multi_way_index.insert(term.clone(), idx);
                    }
                }
            }
        }
    }

    /// Add a one-way synonym mapping where source maps to targets.
    ///
    /// # Example
    /// ```ignore
    /// map.add_one_way("dragon", &["wyrm", "drake"]);
    /// // Searching "dragon" will also match "wyrm" and "drake",
    /// // but searching "wyrm" will NOT match "dragon".
    /// ```
    pub fn add_one_way<S, I, T>(&mut self, source: S, targets: I)
    where
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let source = source.as_ref().to_lowercase();
        let targets: Vec<String> = targets
            .into_iter()
            .map(|t| t.as_ref().to_lowercase())
            .collect();

        if targets.is_empty() {
            return;
        }

        self.one_way_mappings
            .entry(source)
            .or_default()
            .extend(targets);
    }

    /// Expand a single term into all its synonyms.
    ///
    /// Returns the original term plus all synonyms (limited by `max_expansions`).
    pub fn expand_term(&self, term: &str) -> Vec<String> {
        if !self.config.enabled {
            return vec![term.to_string()];
        }

        let term_lower = term.to_lowercase();
        let mut expansions = HashSet::new();

        // Include original if configured
        if self.config.include_original {
            expansions.insert(term.to_string());
        }

        // Check multi-way synonyms
        // Sort to ensure deterministic selection under max_expansions limit,
        // since HashSet iteration order is not guaranteed.
        if let Some(&group_id) = self.multi_way_index.get(&term_lower) {
            let mut sorted: Vec<&String> = self.multi_way_groups[group_id].iter().collect();
            sorted.sort();
            for synonym in sorted {
                if expansions.len() >= self.config.max_expansions {
                    break;
                }
                expansions.insert(synonym.clone());
            }
        }

        // Check one-way synonyms
        if let Some(targets) = self.one_way_mappings.get(&term_lower) {
            for target in targets {
                if expansions.len() >= self.config.max_expansions {
                    break;
                }
                expansions.insert(target.clone());
            }
        }

        let mut result: Vec<String> = expansions.into_iter().collect();
        result.sort();
        result
    }

    /// Expand a full query, returning an `ExpandedQuery` with alternatives at each position.
    ///
    /// # Limitations
    ///
    /// The query is split on whitespace, so multi-word synonym keys (e.g.
    /// "hit points") are matched against individual tokens. A multi-word
    /// synonym like `"hit points" -> "hp"` will only fire if the entire phrase
    /// appears as a single whitespace-delimited token, which in practice means
    /// multi-word synonyms only match via the multi-way group index.
    ///
    /// # Example
    /// ```ignore
    /// let query = map.expand_query("hp recovery items");
    /// // If "hp" has synonyms, the result will have alternatives at position 0.
    /// ```
    pub fn expand_query(&self, query: &str) -> ExpandedQuery {
        let mut expanded = ExpandedQuery::new(query);

        if !self.config.enabled {
            // If disabled, just return the original terms without expansion
            for term in query.split_whitespace() {
                expanded.add_group(TermAlternatives::new(term));
            }
            return expanded;
        }

        for term in query.split_whitespace() {
            let mut group = TermAlternatives::new(term);
            let term_lower = term.to_lowercase();

            // Check multi-way synonyms
            if let Some(&group_id) = self.multi_way_index.get(&term_lower) {
                let mut count = 1; // Original already counted
                // Sort to ensure deterministic expansion order regardless of
                // HashSet iteration order (addresses non-reproducibility concern).
                let mut sorted_synonyms: Vec<&String> =
                    self.multi_way_groups[group_id].iter().collect();
                sorted_synonyms.sort();
                for synonym in sorted_synonyms {
                    if count >= self.config.max_expansions {
                        break;
                    }
                    if !synonym.eq_ignore_ascii_case(term) {
                        group.add_expansion(synonym.clone(), term);
                        count += 1;
                    }
                }
            }

            // Check one-way synonyms
            if let Some(targets) = self.one_way_mappings.get(&term_lower) {
                let mut count = group.alternatives.len();
                for target in targets {
                    if count >= self.config.max_expansions {
                        break;
                    }
                    group.add_expansion(target.clone(), term);
                    count += 1;
                }
            }

            expanded.add_group(group);
        }

        expanded
    }

    /// Load synonyms from a TOML configuration file.
    ///
    /// # TOML Format
    /// ```toml
    /// [synonyms]
    /// multi_way = [
    ///     ["hp", "hit points", "health"],
    ///     ["attack", "atk", "offense"],
    /// ]
    ///
    /// [synonyms.one_way]
    /// dragon = ["wyrm", "drake"]
    /// sword = ["blade", "saber"]
    /// ```
    pub fn load_from_toml(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|e| {
            PreprocessingError::ConfigLoad(format!("Failed to read {}: {}", path.display(), e))
        })?;

        self.load_from_toml_str(&content)
    }

    /// Load synonyms from a TOML string.
    pub fn load_from_toml_str(&mut self, content: &str) -> Result<()> {
        let config: SynonymFileConfig = toml::from_str(content).map_err(|e| {
            PreprocessingError::ConfigLoad(format!("Invalid TOML: {}", e))
        })?;

        // Load multi-way synonyms
        if let Some(synonyms) = config.synonyms {
            if let Some(multi_way) = synonyms.multi_way {
                for group in multi_way {
                    self.add_multi_way(group);
                }
            }

            if let Some(one_way) = synonyms.one_way {
                for (source, targets) in one_way {
                    self.add_one_way(source, targets);
                }
            }
        }

        Ok(())
    }

    /// Load synonyms from a JSON configuration file.
    ///
    /// # JSON Format
    /// ```json
    /// {
    ///     "synonyms": {
    ///         "multi_way": [
    ///             ["hp", "hit points", "health"],
    ///             ["attack", "atk", "offense"]
    ///         ],
    ///         "one_way": {
    ///             "dragon": ["wyrm", "drake"],
    ///             "sword": ["blade", "saber"]
    ///         }
    ///     }
    /// }
    /// ```
    pub fn load_from_json(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|e| {
            PreprocessingError::ConfigLoad(format!("Failed to read {}: {}", path.display(), e))
        })?;

        self.load_from_json_str(&content)
    }

    /// Load synonyms from a JSON string.
    pub fn load_from_json_str(&mut self, content: &str) -> Result<()> {
        let config: SynonymFileConfig = serde_json::from_str(content).map_err(|e| {
            PreprocessingError::ConfigLoad(format!("Invalid JSON: {}", e))
        })?;

        // Load multi-way synonyms
        if let Some(synonyms) = config.synonyms {
            if let Some(multi_way) = synonyms.multi_way {
                for group in multi_way {
                    self.add_multi_way(group);
                }
            }

            if let Some(one_way) = synonyms.one_way {
                for (source, targets) in one_way {
                    self.add_one_way(source, targets);
                }
            }
        }

        Ok(())
    }

    /// Get the number of multi-way synonym groups.
    pub fn multi_way_group_count(&self) -> usize {
        self.multi_way_groups.len()
    }

    /// Get the number of one-way mappings.
    pub fn one_way_mapping_count(&self) -> usize {
        self.one_way_mappings.len()
    }

    /// Check if a term has any synonyms.
    pub fn has_synonyms(&self, term: &str) -> bool {
        let term_lower = term.to_lowercase();
        self.multi_way_index.contains_key(&term_lower)
            || self.one_way_mappings.contains_key(&term_lower)
    }

    /// Rebuild the multi-way index after deserialization.
    ///
    /// This is needed because the index is marked as `#[serde(skip)]`.
    pub fn rebuild_index(&mut self) {
        self.multi_way_index.clear();
        for (group_id, group) in self.multi_way_groups.iter().enumerate() {
            for term in group {
                self.multi_way_index.insert(term.clone(), group_id);
            }
        }
    }
}

/// File configuration structure for loading synonyms.
#[derive(Debug, Deserialize)]
struct SynonymFileConfig {
    synonyms: Option<SynonymSection>,
}

#[derive(Debug, Deserialize)]
struct SynonymSection {
    multi_way: Option<Vec<Vec<String>>>,
    one_way: Option<HashMap<String, Vec<String>>>,
}

/// Build a comprehensive TTRPG synonym map with default game terms.
///
/// Includes:
/// - Stat abbreviations (str, dex, con, int, wis, cha, hp, ac, dc, cr, xp)
/// - Game mechanics (aoo, crit, nat 20, tpk, dm, gm, pc, npc, bbeg)
/// - Condition terms (prone, grappled, stunned, frightened, poisoned, etc.)
/// - Book abbreviations (phb, dmg, mm, xge, tce)
/// - Damage types (fire, cold, lightning, necrotic, radiant, psychic)
/// - Creature types (undead, dragon, devil, fiend)
pub fn build_default_ttrpg_synonyms() -> SynonymMap {
    let mut map = SynonymMap::with_config(SynonymConfig {
        max_expansions: 5,
        enabled: true,
        include_original: true,
    });

    // === Stat abbreviations (multi-way) ===
    map.add_multi_way(&["hp", "hit points", "health", "life"]);
    map.add_multi_way(&["ac", "armor class", "armour class"]);
    map.add_multi_way(&["str", "strength"]);
    map.add_multi_way(&["dex", "dexterity"]);
    map.add_multi_way(&["con", "constitution"]);
    map.add_multi_way(&["int", "intelligence"]);
    map.add_multi_way(&["wis", "wisdom"]);
    map.add_multi_way(&["cha", "charisma"]);
    map.add_multi_way(&["dc", "difficulty class"]);
    map.add_multi_way(&["cr", "challenge rating"]);
    map.add_multi_way(&["xp", "experience points", "exp"]);
    map.add_multi_way(&["pp", "passive perception"]);

    // === Game mechanics ===
    map.add_multi_way(&["aoo", "opportunity attack", "attack of opportunity"]);
    map.add_multi_way(&["tpk", "total party kill"]);
    map.add_multi_way(&["nat 20", "natural 20", "critical hit", "crit"]);
    map.add_multi_way(&["nat 1", "natural 1", "critical failure", "fumble"]);
    map.add_multi_way(&["dm", "dungeon master", "game master", "gm"]);
    map.add_multi_way(&["pc", "player character"]);
    map.add_multi_way(&["npc", "non-player character"]);
    map.add_multi_way(&["bbeg", "big bad evil guy", "main villain"]);

    // === Condition synonyms ===
    map.add_multi_way(&["prone", "knocked down", "on the ground"]);
    map.add_multi_way(&["grappled", "grabbed", "held"]);
    map.add_multi_way(&["stunned", "stun", "dazed"]);
    map.add_multi_way(&["frightened", "scared", "afraid", "fear"]);
    map.add_multi_way(&["poisoned", "poison", "toxic"]);
    map.add_multi_way(&["invisible", "invisibility", "unseen"]);
    map.add_multi_way(&["blinded", "blind"]);
    map.add_multi_way(&["deafened", "deaf"]);
    map.add_multi_way(&["charmed", "charm"]);
    map.add_multi_way(&["paralyzed", "paralysis"]);
    map.add_multi_way(&["petrified", "turned to stone"]);
    map.add_multi_way(&["restrained", "restrain"]);
    map.add_multi_way(&["incapacitated", "incapacitate"]);
    map.add_multi_way(&["unconscious", "knocked out", "ko"]);
    map.add_multi_way(&["exhaustion", "exhausted", "fatigue"]);

    // === Book / source abbreviations ===
    map.add_multi_way(&["phb", "player's handbook", "players handbook"]);
    map.add_multi_way(&["dmg", "dungeon master's guide", "dungeon masters guide"]);
    map.add_multi_way(&["mm", "monster manual"]);
    map.add_multi_way(&["xge", "xanathar's guide", "xanathars guide to everything"]);
    map.add_multi_way(&["tce", "tasha's cauldron", "tashas cauldron of everything"]);
    map.add_multi_way(&["vgm", "volo's guide", "volos guide to monsters"]);
    map.add_multi_way(&["mtof", "mordenkainen's tome", "mordenkainens tome of foes"]);
    map.add_multi_way(&["scag", "sword coast adventurers guide"]);

    // === Damage types (one-way - "fire" shouldn't expand to "fire damage") ===
    map.add_one_way("fire damage", &["flame damage", "burning"]);
    map.add_one_way("cold damage", &["frost damage", "ice damage", "freezing"]);
    map.add_one_way("lightning damage", &["electric damage", "shock damage"]);
    map.add_one_way("thunder damage", &["sonic damage"]);
    map.add_one_way("necrotic damage", &["death damage", "dark damage"]);
    map.add_one_way("radiant damage", &["holy damage", "light damage"]);
    map.add_one_way("psychic damage", &["mental damage", "mind damage"]);
    map.add_one_way("force damage", &["magical damage"]);
    map.add_one_way("poison damage", &["toxic damage"]);
    map.add_one_way("acid damage", &["corrosive damage"]);

    // === Creature types (one-way hierarchies) ===
    map.add_one_way("dragon", &["wyrm", "drake", "wyvern"]);
    map.add_one_way("devil", &["fiend", "demon", "daemon"]);
    map.add_one_way("undead", &["zombie", "skeleton", "vampire", "lich", "ghost", "wraith"]);
    map.add_one_way("giant", &["ogre", "troll", "cyclops"]);
    map.add_one_way("goblinoid", &["goblin", "hobgoblin", "bugbear"]);

    // === Action types ===
    map.add_multi_way(&["bonus action", "ba"]);
    map.add_multi_way(&["reaction", "rxn"]);
    map.add_multi_way(&["concentration", "conc"]);
    map.add_multi_way(&["advantage", "adv"]);
    map.add_multi_way(&["disadvantage", "disadv", "dis"]);

    // === Spell components ===
    map.add_multi_way(&["verbal", "v"]);
    map.add_multi_way(&["somatic", "s"]);
    map.add_multi_way(&["material", "m"]);

    // === Common misnomers / player slang ===
    map.add_one_way("healing word", &["cure wounds"]); // players often confuse
    map.add_one_way("magic missile", &["magic missle"]); // common misspelling as synonym

    map
}

/// A synonym map that supports campaign-specific overlays.
///
/// Base synonyms apply to all searches, while campaign synonyms
/// are only active when searching within that campaign.
///
/// This implements REQ-QP-002.6 for campaign-scoped synonym support.
#[derive(Debug, Clone)]
pub struct CampaignScopedSynonyms {
    /// Base synonyms that apply globally.
    base: SynonymMap,
    /// Campaign-specific synonym overlays (campaign_id -> synonyms).
    campaign_overlays: HashMap<String, SynonymMap>,
}

impl CampaignScopedSynonyms {
    /// Create a new campaign-scoped synonym map with the given base synonyms.
    pub fn new(base: SynonymMap) -> Self {
        Self {
            base,
            campaign_overlays: HashMap::new(),
        }
    }

    /// Create a new campaign-scoped synonym map with default TTRPG synonyms as the base.
    pub fn with_default_ttrpg_base() -> Self {
        Self::new(build_default_ttrpg_synonyms())
    }

    /// Add a campaign-specific synonym overlay.
    ///
    /// These synonyms will only be active when searching within the specified campaign.
    ///
    /// # Example
    /// ```ignore
    /// let mut scoped = CampaignScopedSynonyms::new(base_map);
    ///
    /// // Add campaign-specific NPCs and locations
    /// let mut campaign_synonyms = SynonymMap::new();
    /// campaign_synonyms.add_multi_way(&["the blackstaff", "vajra safahr", "open lord"]);
    ///
    /// scoped.add_campaign_overlay("waterdeep-dragon-heist", campaign_synonyms);
    /// ```
    pub fn add_campaign_overlay(&mut self, campaign_id: impl Into<String>, synonyms: SynonymMap) {
        self.campaign_overlays.insert(campaign_id.into(), synonyms);
    }

    /// Remove a campaign overlay.
    ///
    /// Returns the removed synonym map if it existed.
    pub fn remove_campaign_overlay(&mut self, campaign_id: &str) -> Option<SynonymMap> {
        self.campaign_overlays.remove(campaign_id)
    }

    /// Check if a campaign overlay exists.
    pub fn has_campaign_overlay(&self, campaign_id: &str) -> bool {
        self.campaign_overlays.contains_key(campaign_id)
    }

    /// Get a reference to a campaign's overlay (without base).
    pub fn get_campaign_overlay(&self, campaign_id: &str) -> Option<&SynonymMap> {
        self.campaign_overlays.get(campaign_id)
    }

    /// Get a mutable reference to a campaign's overlay.
    pub fn get_campaign_overlay_mut(&mut self, campaign_id: &str) -> Option<&mut SynonymMap> {
        self.campaign_overlays.get_mut(campaign_id)
    }

    /// Get the base synonym map.
    pub fn base(&self) -> &SynonymMap {
        &self.base
    }

    /// Get a mutable reference to the base synonym map.
    pub fn base_mut(&mut self) -> &mut SynonymMap {
        &mut self.base
    }

    /// Get effective synonym map for a campaign (base + overlay merged).
    ///
    /// If `campaign_id` is `None`, returns a clone of the base map.
    /// If the campaign has an overlay, returns a merged map where campaign
    /// synonyms take precedence.
    pub fn for_campaign(&self, campaign_id: Option<&str>) -> SynonymMap {
        // Fast path: no campaign overlay, return a clone of base without merging.
        let overlay = campaign_id.and_then(|id| self.campaign_overlays.get(id));
        let Some(overlay) = overlay else {
            return self.base.clone();
        };

        let mut merged = self.base.clone();

        // Multi-way groups from overlay
        for group in &overlay.multi_way_groups {
            let terms: Vec<&str> = group.iter().map(|s| s.as_str()).collect();
            merged.add_multi_way(terms);
        }

        // One-way mappings from overlay
        for (source, targets) in &overlay.one_way_mappings {
            merged.add_one_way(source, targets.iter().map(|s| s.as_str()));
        }

        merged
    }

    /// Expand a term using campaign-scoped synonyms.
    ///
    /// First checks campaign-specific synonyms (if campaign_id provided),
    /// then falls back to base synonyms.
    pub fn expand_term(&self, term: &str, campaign_id: Option<&str>) -> Vec<String> {
        let effective_map = self.for_campaign(campaign_id);
        effective_map.expand_term(term)
    }

    /// Expand a query using campaign-scoped synonyms.
    pub fn expand_query(&self, query: &str, campaign_id: Option<&str>) -> ExpandedQuery {
        let effective_map = self.for_campaign(campaign_id);
        effective_map.expand_query(query)
    }

    /// List all campaign IDs with overlays.
    pub fn campaign_ids(&self) -> impl Iterator<Item = &str> {
        self.campaign_overlays.keys().map(|s| s.as_str())
    }

    /// Get the number of campaign overlays.
    pub fn overlay_count(&self) -> usize {
        self.campaign_overlays.len()
    }
}

impl Default for CampaignScopedSynonyms {
    fn default() -> Self {
        Self::new(SynonymMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_way_synonyms() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "hit points", "health"]);

        let expansions = map.expand_term("hp");
        assert!(expansions.contains(&"hp".to_string()));
        assert!(expansions.contains(&"hit points".to_string()));
        assert!(expansions.contains(&"health".to_string()));

        // Multi-way should work in reverse
        let expansions = map.expand_term("health");
        assert!(expansions.contains(&"hp".to_string()));
    }

    #[test]
    fn test_one_way_synonyms() {
        let mut map = SynonymMap::new();
        map.add_one_way("dragon", &["wyrm", "drake"]);

        let expansions = map.expand_term("dragon");
        assert!(expansions.contains(&"dragon".to_string()));
        assert!(expansions.contains(&"wyrm".to_string()));
        assert!(expansions.contains(&"drake".to_string()));

        // One-way should NOT work in reverse
        let expansions = map.expand_term("wyrm");
        assert_eq!(expansions.len(), 1);
        assert!(expansions.contains(&"wyrm".to_string()));
    }

    #[test]
    fn test_expand_query() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "hit points", "health"]);
        map.add_one_way("dragon", &["wyrm"]);

        let expanded = map.expand_query("hp dragon");

        assert!(expanded.has_expansions);
        assert_eq!(expanded.term_groups.len(), 2);

        // First group should have hp and its synonyms
        let hp_terms = expanded.term_groups[0].all_terms();
        assert!(hp_terms.contains(&"hp"));

        // Second group should have dragon and wyrm
        let dragon_terms = expanded.term_groups[1].all_terms();
        assert!(dragon_terms.contains(&"dragon"));
        assert!(dragon_terms.contains(&"wyrm"));
    }

    #[test]
    fn test_fts5_output() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "health"]);

        let expanded = map.expand_query("hp recovery");
        let fts5 = expanded.to_fts5_match();

        // Should contain OR for hp/health, AND for recovery
        assert!(fts5.contains("OR"));
        assert!(fts5.contains("AND"));
        assert!(fts5.contains("recovery"));
    }

    #[test]
    fn test_surrealdb_output() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "health"]);

        let expanded = map.expand_query("hp recovery");
        let surrealdb = expanded.to_surrealdb_fts("content", 0);

        assert!(surrealdb.contains("content @@"));
        assert!(surrealdb.contains("OR"));
        assert!(surrealdb.contains("AND"));
    }

    #[test]
    fn test_max_expansions() {
        let mut map = SynonymMap::with_config(SynonymConfig {
            max_expansions: 2,
            enabled: true,
            include_original: true,
        });
        map.add_multi_way(&["a", "b", "c", "d", "e"]);

        let expansions = map.expand_term("a");
        assert!(expansions.len() <= 2);
    }

    #[test]
    fn test_disabled_expansion() {
        let mut map = SynonymMap::with_config(SynonymConfig {
            enabled: false,
            ..Default::default()
        });
        map.add_multi_way(&["hp", "health"]);

        let expansions = map.expand_term("hp");
        assert_eq!(expansions, vec!["hp".to_string()]);
    }

    #[test]
    fn test_case_insensitivity() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["HP", "Health"]);

        let expansions = map.expand_term("hp");
        assert!(expansions.len() > 1);

        let expansions = map.expand_term("HEALTH");
        assert!(expansions.len() > 1);
    }

    #[test]
    fn test_load_toml() {
        let toml = r#"
            [synonyms]
            multi_way = [
                ["hp", "hit points", "health"],
            ]

            [synonyms.one_way]
            dragon = ["wyrm", "drake"]
        "#;

        let mut map = SynonymMap::new();
        map.load_from_toml_str(toml).unwrap();

        assert_eq!(map.multi_way_group_count(), 1);
        assert_eq!(map.one_way_mapping_count(), 1);

        assert!(map.has_synonyms("hp"));
        assert!(map.has_synonyms("dragon"));
    }

    #[test]
    fn test_load_json() {
        let json = r#"{
            "synonyms": {
                "multi_way": [
                    ["hp", "hit points", "health"]
                ],
                "one_way": {
                    "dragon": ["wyrm", "drake"]
                }
            }
        }"#;

        let mut map = SynonymMap::new();
        map.load_from_json_str(json).unwrap();

        assert_eq!(map.multi_way_group_count(), 1);
        assert_eq!(map.one_way_mapping_count(), 1);
    }

    #[test]
    fn test_expanded_string() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "health"]);

        let expanded = map.expand_query("hp recovery");
        let expanded_str = expanded.to_expanded_string();

        // Should show alternatives in brackets
        assert!(expanded_str.contains("["));
        assert!(expanded_str.contains("]"));
        assert!(expanded_str.contains("recovery")); // No brackets for unmodified term
    }

    // ========================================
    // TTRPG Default Synonyms Tests
    // ========================================

    #[test]
    fn test_default_ttrpg_synonyms_stat_abbreviations() {
        let map = build_default_ttrpg_synonyms();

        // Test HP synonyms
        let hp_expansions = map.expand_term("hp");
        assert!(hp_expansions.contains(&"hp".to_string()));
        assert!(hp_expansions.contains(&"hit points".to_string()));

        // Test AC synonyms
        let ac_expansions = map.expand_term("ac");
        assert!(ac_expansions.contains(&"armor class".to_string()));

        // Test ability score abbreviations
        assert!(map.has_synonyms("str"));
        assert!(map.has_synonyms("dex"));
        assert!(map.has_synonyms("con"));
        assert!(map.has_synonyms("int"));
        assert!(map.has_synonyms("wis"));
        assert!(map.has_synonyms("cha"));

        // Test DC and CR
        assert!(map.has_synonyms("dc"));
        assert!(map.has_synonyms("cr"));
        assert!(map.has_synonyms("xp"));
    }

    #[test]
    fn test_default_ttrpg_synonyms_game_mechanics() {
        let map = build_default_ttrpg_synonyms();

        // Test DM/GM synonyms (multi-way)
        let dm_expansions = map.expand_term("dm");
        assert!(dm_expansions.contains(&"dungeon master".to_string()));
        assert!(dm_expansions.contains(&"game master".to_string()));
        assert!(dm_expansions.contains(&"gm".to_string()));

        // Test PC/NPC
        assert!(map.has_synonyms("pc"));
        assert!(map.has_synonyms("npc"));
        assert!(map.has_synonyms("bbeg"));

        // Test TPK
        let tpk_expansions = map.expand_term("tpk");
        assert!(tpk_expansions.contains(&"total party kill".to_string()));
    }

    #[test]
    fn test_default_ttrpg_synonyms_conditions() {
        let map = build_default_ttrpg_synonyms();

        // Test various conditions
        assert!(map.has_synonyms("prone"));
        assert!(map.has_synonyms("grappled"));
        assert!(map.has_synonyms("stunned"));
        assert!(map.has_synonyms("frightened"));
        assert!(map.has_synonyms("poisoned"));
        assert!(map.has_synonyms("invisible"));
        assert!(map.has_synonyms("unconscious"));

        // Test specific expansion
        let frightened = map.expand_term("frightened");
        assert!(frightened.contains(&"scared".to_string()));
        assert!(frightened.contains(&"afraid".to_string()));
    }

    #[test]
    fn test_default_ttrpg_synonyms_book_abbreviations() {
        let map = build_default_ttrpg_synonyms();

        // Test PHB
        let phb_expansions = map.expand_term("phb");
        assert!(phb_expansions.contains(&"player's handbook".to_string()));

        // Test DMG
        let dmg_expansions = map.expand_term("dmg");
        assert!(dmg_expansions.contains(&"dungeon master's guide".to_string()));

        // Test other books
        assert!(map.has_synonyms("mm"));
        assert!(map.has_synonyms("xge"));
        assert!(map.has_synonyms("tce"));
    }

    #[test]
    fn test_default_ttrpg_synonyms_damage_types_one_way() {
        let map = build_default_ttrpg_synonyms();

        // One-way: "fire damage" expands to "flame damage", "burning"
        let fire_expansions = map.expand_term("fire damage");
        assert!(fire_expansions.contains(&"fire damage".to_string()));
        assert!(fire_expansions.contains(&"flame damage".to_string()));
        assert!(fire_expansions.contains(&"burning".to_string()));

        // But "flame damage" should NOT expand to "fire damage" (one-way)
        let flame_expansions = map.expand_term("flame damage");
        assert_eq!(flame_expansions.len(), 1);
        assert!(flame_expansions.contains(&"flame damage".to_string()));
    }

    #[test]
    fn test_default_ttrpg_synonyms_creature_types_one_way() {
        let map = build_default_ttrpg_synonyms();

        // One-way: "dragon" expands to subtypes
        let dragon_expansions = map.expand_term("dragon");
        assert!(dragon_expansions.contains(&"wyrm".to_string()));
        assert!(dragon_expansions.contains(&"drake".to_string()));

        // But "wyrm" should NOT expand to "dragon" (one-way)
        let wyrm_expansions = map.expand_term("wyrm");
        assert_eq!(wyrm_expansions.len(), 1);

        // Test undead hierarchy
        let undead_expansions = map.expand_term("undead");
        assert!(undead_expansions.contains(&"zombie".to_string()));
        assert!(undead_expansions.contains(&"skeleton".to_string()));
        assert!(undead_expansions.contains(&"vampire".to_string()));
    }

    #[test]
    fn test_default_ttrpg_synonyms_action_types() {
        let map = build_default_ttrpg_synonyms();

        // Test advantage/disadvantage
        let adv_expansions = map.expand_term("adv");
        assert!(adv_expansions.contains(&"advantage".to_string()));

        let disadv_expansions = map.expand_term("disadv");
        assert!(disadv_expansions.contains(&"disadvantage".to_string()));

        // Test bonus action
        let ba_expansions = map.expand_term("ba");
        assert!(ba_expansions.contains(&"bonus action".to_string()));
    }

    // ========================================
    // Campaign-Scoped Synonyms Tests
    // ========================================

    #[test]
    fn test_campaign_scoped_synonyms_base_only() {
        let mut base = SynonymMap::new();
        base.add_multi_way(&["hp", "hit points"]);

        let scoped = CampaignScopedSynonyms::new(base);

        // Without campaign, should use base
        let expansions = scoped.expand_term("hp", None);
        assert!(expansions.contains(&"hp".to_string()));
        assert!(expansions.contains(&"hit points".to_string()));
    }

    #[test]
    fn test_campaign_scoped_synonyms_overlay_merging() {
        let mut base = SynonymMap::new();
        base.add_multi_way(&["hp", "hit points"]);

        let mut scoped = CampaignScopedSynonyms::new(base);

        // Add campaign-specific synonyms
        let mut campaign_map = SynonymMap::new();
        campaign_map.add_multi_way(&["the blackstaff", "vajra safahr"]);
        campaign_map.add_one_way("xanathar", &["the beholder", "fish lover"]);

        scoped.add_campaign_overlay("waterdeep", campaign_map);

        // Without campaign - only base synonyms
        let no_campaign = scoped.expand_term("the blackstaff", None);
        assert_eq!(no_campaign.len(), 1);
        assert!(no_campaign.contains(&"the blackstaff".to_string()));

        // With campaign - should have campaign synonyms
        let with_campaign = scoped.expand_term("the blackstaff", Some("waterdeep"));
        assert!(with_campaign.contains(&"vajra safahr".to_string()));

        // Base synonyms should still work with campaign
        let hp_with_campaign = scoped.expand_term("hp", Some("waterdeep"));
        assert!(hp_with_campaign.contains(&"hit points".to_string()));
    }

    #[test]
    fn test_campaign_scoped_synonyms_query_expansion() {
        let mut base = SynonymMap::new();
        base.add_multi_way(&["hp", "hit points"]);

        let mut scoped = CampaignScopedSynonyms::new(base);

        let mut campaign_map = SynonymMap::new();
        campaign_map.add_multi_way(&["trollskull", "tavern", "manor"]);
        scoped.add_campaign_overlay("waterdeep", campaign_map);

        // Expand query with campaign
        let expanded = scoped.expand_query("trollskull hp", Some("waterdeep"));

        assert!(expanded.has_expansions);
        assert_eq!(expanded.term_groups.len(), 2);

        // First term should have campaign synonyms
        let trollskull_terms = expanded.term_groups[0].all_terms();
        assert!(trollskull_terms.contains(&"tavern"));

        // Second term should have base synonyms
        let hp_terms = expanded.term_groups[1].all_terms();
        assert!(hp_terms.contains(&"hit points"));
    }

    #[test]
    fn test_campaign_scoped_synonyms_remove_overlay() {
        let base = SynonymMap::new();
        let mut scoped = CampaignScopedSynonyms::new(base);

        let mut campaign_map = SynonymMap::new();
        campaign_map.add_multi_way(&["test", "example"]);
        scoped.add_campaign_overlay("test-campaign", campaign_map);

        assert!(scoped.has_campaign_overlay("test-campaign"));
        assert_eq!(scoped.overlay_count(), 1);

        let removed = scoped.remove_campaign_overlay("test-campaign");
        assert!(removed.is_some());
        assert!(!scoped.has_campaign_overlay("test-campaign"));
        assert_eq!(scoped.overlay_count(), 0);
    }

    #[test]
    fn test_campaign_scoped_synonyms_for_campaign_returns_merged() {
        let mut base = SynonymMap::new();
        base.add_multi_way(&["sword", "blade"]);

        let mut scoped = CampaignScopedSynonyms::new(base);

        let mut campaign_map = SynonymMap::new();
        campaign_map.add_multi_way(&["azuredge", "legendary sword"]);
        scoped.add_campaign_overlay("waterdeep", campaign_map);

        // Get merged map for campaign
        let merged = scoped.for_campaign(Some("waterdeep"));

        // Should have both base and campaign synonyms
        assert!(merged.has_synonyms("sword"));
        assert!(merged.has_synonyms("azuredge"));
    }

    #[test]
    fn test_campaign_scoped_synonyms_with_default_ttrpg_base() {
        let mut scoped = CampaignScopedSynonyms::with_default_ttrpg_base();

        // Should have all the TTRPG defaults
        let hp_expansions = scoped.expand_term("hp", None);
        assert!(hp_expansions.contains(&"hit points".to_string()));

        // Add campaign overlay
        let mut campaign_map = SynonymMap::new();
        campaign_map.add_multi_way(&["laeral", "open lord", "lady silverhand"]);
        scoped.add_campaign_overlay("waterdeep", campaign_map);

        // Campaign should have both TTRPG defaults and campaign-specific
        let expanded = scoped.expand_query("hp laeral", Some("waterdeep"));

        let hp_terms = expanded.term_groups[0].all_terms();
        assert!(hp_terms.contains(&"hit points"));

        let laeral_terms = expanded.term_groups[1].all_terms();
        assert!(laeral_terms.contains(&"open lord"));
    }

    #[test]
    fn test_campaign_scoped_synonyms_campaign_ids() {
        let base = SynonymMap::new();
        let mut scoped = CampaignScopedSynonyms::new(base);

        scoped.add_campaign_overlay("campaign-a", SynonymMap::new());
        scoped.add_campaign_overlay("campaign-b", SynonymMap::new());
        scoped.add_campaign_overlay("campaign-c", SynonymMap::new());

        let ids: Vec<&str> = scoped.campaign_ids().collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"campaign-a"));
        assert!(ids.contains(&"campaign-b"));
        assert!(ids.contains(&"campaign-c"));
    }

    #[test]
    fn test_campaign_scoped_synonyms_nonexistent_campaign() {
        let mut base = SynonymMap::new();
        base.add_multi_way(&["hp", "hit points"]);

        let scoped = CampaignScopedSynonyms::new(base);

        // Using nonexistent campaign should just return base synonyms
        let expansions = scoped.expand_term("hp", Some("nonexistent-campaign"));
        assert!(expansions.contains(&"hit points".to_string()));
    }

    #[test]
    fn test_synonym_map_roundtrip_preserves_multi_way() {
        let mut map = SynonymMap::new();
        map.add_multi_way(&["hp", "hit points", "health"]);
        map.add_one_way("dragon", &["wyrm", "drake"]);

        // Serialize and deserialize (round-trip)
        let json = serde_json::to_string(&map).unwrap();
        let deserialized: SynonymMap = serde_json::from_str(&json).unwrap();

        // Multi-way synonyms should still work after round-trip
        let expansions = deserialized.expand_term("hp");
        assert!(expansions.contains(&"hp".to_string()));
        assert!(expansions.contains(&"hit points".to_string()));
        assert!(expansions.contains(&"health".to_string()));

        // One-way should also work
        let dragon_exp = deserialized.expand_term("dragon");
        assert!(dragon_exp.contains(&"wyrm".to_string()));
    }
}
