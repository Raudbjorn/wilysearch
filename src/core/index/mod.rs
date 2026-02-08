mod documents;
mod facets_similar;
mod hybrid;
mod search;
mod settings_ops;
mod updates;
mod vectors;

pub use documents::DocumentsResult;

use milli::FieldId;
use milli::FieldsIdsMap;
use milli::tokenizer::Language;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::core::error::{Error, Result};
use crate::core::vector::VectorStore;

/// Parse a filter `Value` into a filter expression string.
///
/// Returns `Ok(None)` for empty/null values.
/// Returns `Err(InvalidFilter)` for unsupported shapes.
pub(crate) fn parse_filter_to_string(filter_val: &Value) -> Result<Option<String>> {
    match filter_val {
        Value::String(s) if s.is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        Value::Array(arr) if arr.is_empty() => Ok(None),
        Value::Array(arr) => {
            let mut and_clauses = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::String(s) => and_clauses.push(s.clone()),
                    Value::Array(inner) => {
                        let or_parts: Vec<&str> = inner
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect();
                        if or_parts.is_empty() {
                            continue;
                        }
                        if or_parts.len() == 1 {
                            and_clauses.push(or_parts[0].to_string());
                        } else {
                            and_clauses.push(format!("({})", or_parts.join(" OR ")));
                        }
                    }
                    _ => {
                        return Err(Error::InvalidFilter(
                            "filter array elements must be strings or arrays of strings".to_string(),
                        ));
                    }
                }
            }
            if and_clauses.is_empty() {
                Ok(None)
            } else {
                Ok(Some(and_clauses.join(" AND ")))
            }
        }
        Value::Null => Ok(None),
        _ => Err(Error::InvalidFilter(
            "filter must be a string or array".to_string(),
        )),
    }
}

/// Convert a primary key JSON value to the string form milli uses for external ID mapping.
pub(crate) fn pk_value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// A single Meilisearch index backed by milli's LMDB storage.
///
/// Provides methods for adding, updating, deleting, and searching documents,
/// as well as reading and writing index settings.
///
/// Obtain an `Index` via [`Meilisearch::create_index`](crate::Meilisearch::create_index)
/// or [`Meilisearch::get_index`](crate::Meilisearch::get_index).
pub struct Index {
    pub(crate) inner: milli::Index,
    pub(crate) vector_store: Option<Arc<dyn VectorStore>>,
}

impl Index {
    /// Wrap a raw milli index with an optional external vector store.
    pub fn new(inner: milli::Index, vector_store: Option<Arc<dyn VectorStore>>) -> Self {
        Self { inner, vector_store }
    }

    /// Determine which fields to display based on index settings and query parameters.
    pub(crate) fn get_displayed_fields(
        &self,
        rtxn: &milli::heed::RoTxn<'_>,
        fields_ids_map: &FieldsIdsMap,
        attributes_to_retrieve: Option<&BTreeSet<String>>,
    ) -> Result<Vec<FieldId>> {
        // If specific attributes are requested, use those
        if let Some(attrs) = attributes_to_retrieve {
            let field_ids: Vec<FieldId> = attrs
                .iter()
                .filter_map(|name| fields_ids_map.id(name))
                .collect();
            return Ok(field_ids);
        }

        // Otherwise, use the displayed fields from index settings
        let displayed_fields = self
            .inner
            .displayed_fields_ids(rtxn)
            .map_err(Error::Milli)?
            .map(|fields| fields.into_iter().collect::<Vec<_>>())
            .unwrap_or_else(|| fields_ids_map.ids().collect());

        Ok(displayed_fields)
    }

    /// Parse locale strings into milli Language values.
    pub(crate) fn parse_locales(&self, locales: Option<&[String]>) -> Result<Option<Vec<Language>>> {
        match locales {
            None => Ok(None),
            Some(locale_strs) => {
                let mut langs = Vec::with_capacity(locale_strs.len());
                for s in locale_strs {
                    let locale: meilisearch_types::locales::Locale = s
                        .parse()
                        .map_err(|_| Error::Internal(format!("Unknown locale: {s}")))?;
                    langs.push(Language::from(locale));
                }
                Ok(Some(langs))
            }
        }
    }
}
