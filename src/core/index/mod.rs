mod documents;
mod facets_similar;
mod hybrid;
mod search;
mod settings_ops;
mod updates;
mod vectors;

pub use documents::DocumentsResult;

use milli::FieldsIdsMap;
use milli::tokenizer::Language;
use serde_json::Value;
use std::sync::Arc;

use crate::core::error::{Error, Result};
use crate::core::vector::VectorStore;

/// Parse JSON filters with milli, preserving grouping and rejecting invalid values.
pub(crate) fn parse_local_filter(value: &Value) -> Result<Option<milli::IndexFilter>> {
    if value.is_null() {
        return Ok(None);
    }
    milli::Filter::from_json(value)?
        .map(local_filter)
        .transpose()
}

fn local_filter(filter: milli::Filter) -> Result<milli::IndexFilter> {
    milli::IndexFilter::from_filter_without_foreign(filter).map_err(|_| {
        Error::InvalidFilter("Foreign filters require the engine's cross-index search API".into())
    })
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
    pub(crate) uid: String,
    pub(crate) ip_policy: http_client::policy::IpPolicy,
    pub(crate) vector_store: Option<Arc<dyn VectorStore>>,
}

impl Index {
    pub(crate) fn make_document(
        &self,
        txn: &milli::heed::RoTxn<'_>,
        fields: &FieldsIdsMap,
        id: u32,
        requested: Option<&[String]>,
        vectors: bool,
        respect_displayed: bool,
    ) -> Result<Value> {
        let displayed = if respect_displayed {
            self.inner.displayed_fields(txn)?
        } else {
            None
        };
        let requested = requested.filter(|names| !names.iter().any(|name| name == "*"));
        let all: Vec<String> = fields.names().map(str::to_owned).collect();
        let selectors = requested.unwrap_or(&all);
        let raw = self.inner.document(txn, id)?;
        let mut doc = milli::make_document(raw, fields, selectors)?;
        let vectors_visible = displayed
            .as_ref()
            .is_none_or(|names| names.iter().any(|n| *n == "_vectors" || *n == "*"));
        if let Some(displayed) = displayed {
            let allowed: Vec<String> = displayed.into_iter().map(str::to_owned).collect();
            let visible = milli::make_document(raw, fields, &allowed)?;
            // Projection may select nested paths; intersect recursively with displayed data.
            fn intersect(value: &mut Value, allowed: &Value) {
                match (value, allowed) {
                    (Value::Object(v), Value::Object(a)) => v.retain(|key, value| {
                        if let Some(allowed) = a.get(key) {
                            intersect(value, allowed);
                            true
                        } else {
                            false
                        }
                    }),
                    (Value::Array(v), Value::Array(a)) => {
                        for (value, allowed) in v.iter_mut().zip(a) {
                            intersect(value, allowed);
                        }
                    }
                    _ => {}
                }
            }
            let mut projected = Value::Object(doc);
            intersect(&mut projected, &Value::Object(visible));
            doc = projected.as_object().cloned().unwrap_or_default();
        }
        let raw_vectors = doc.remove("_vectors");
        if vectors && vectors_visible {
            let mut entries = raw_vectors
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            for (name, data) in self.inner.embeddings(txn, id)? {
                entries.insert(name, serde_json::json!({"embeddings": data.embeddings, "regenerate": data.regenerate}));
            }
            doc.insert("_vectors".into(), entries.into());
        }
        Ok(Value::Object(doc))
    }
    /// Wrap a raw milli index with an optional external vector store.
    pub fn new(inner: milli::Index, vector_store: Option<Arc<dyn VectorStore>>) -> Self {
        Self {
            inner,
            vector_store,
            uid: String::new(),
            ip_policy: http_client::policy::IpPolicy::deny_all_local_ips(),
        }
    }

    /// Parse locale strings into milli Language values.
    pub(crate) fn parse_locales(
        &self,
        locales: Option<&[String]>,
    ) -> Result<Option<Vec<Language>>> {
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
