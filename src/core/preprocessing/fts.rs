//! FTS (Full-Text Search) query generation for expanded queries.
//!
//! This module provides methods on [`ExpandedQuery`] for generating
//! database-specific full-text search expressions:
//! - SurrealDB FTS (`@@` operator)
//! - SQLite FTS5 (`MATCH` expression)
//! - Human-readable expanded string

use super::synonyms::ExpandedQuery;

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

impl ExpandedQuery {
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
                            let escaped = escape_fts5_term(t);
                            if t.contains(' ')
                                && !(escaped.starts_with('"') && escaped.ends_with('"'))
                            {
                                format!("\"{}\"", escaped)
                            } else {
                                escaped
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
