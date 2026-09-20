use std::collections::{BTreeSet, HashMap};

use milli::{Filter, IndexFilter, IndexFilterCondition, TokenLike};
use serde_json::Value;

use super::Meilisearch;
use crate::core::{Error, Result, SearchQuery, SearchResult};

impl Meilisearch {
    pub fn get_documents(
        &self,
        uid: &str,
        options: &crate::core::GetDocumentsOptions,
    ) -> Result<crate::core::index::DocumentsResult> {
        let filter = self.resolve_filter(uid, options.filter.as_ref())?;
        self.get_index(uid)?.documents_with_filter(options, filter)
    }

    pub fn facet_search(
        &self,
        uid: &str,
        query: &crate::core::FacetSearchQuery,
    ) -> Result<crate::core::FacetSearchResult> {
        validate_threshold(query.ranking_score_threshold)?;
        let filter = self.resolve_filter(uid, query.filter.as_ref())?;
        self.get_index(uid)?.facet_search_with_filter(query, filter)
    }

    pub fn similar(
        &self,
        uid: &str,
        query: &crate::core::SimilarQuery,
    ) -> Result<crate::core::SimilarResult> {
        validate_threshold(query.ranking_score_threshold)?;
        let filter = self.resolve_filter(uid, query.filter.as_ref())?;
        let mut query = query.clone();
        let hydrate = self.get_experimental_features().foreign_keys;
        let projection = if hydrate {
            query.attributes_to_retrieve.take()
        } else {
            None
        };
        let mut result = self.get_index(uid)?.similar_with_filter(&query, filter)?;
        if hydrate {
            for hit in &mut result.hits {
                self.hydrate(uid, &mut hit.document)?;
                project(
                    &mut hit.document,
                    projection.as_ref(),
                    query.retrieve_vectors,
                );
            }
        }
        Ok(result)
    }

    /// Search local indexes with foreign filters, hydration, and dynamic rules.
    pub fn search(&self, uid: &str, query: &SearchQuery) -> Result<SearchResult> {
        if query.media.is_some() && !self.get_experimental_features().multimodal {
            return Err(Error::ExperimentalFeatureNotEnabled("multimodal".into()));
        }
        let index = self.get_index(uid)?;
        let filter = self.resolve_filter(uid, query.filter.as_ref())?;
        let mut query = query.clone();
        query.filter = None;
        let hydrate = self.get_experimental_features().foreign_keys;
        let projection = if hydrate {
            query.attributes_to_retrieve.take()
        } else {
            None
        };
        let rules = self.search_rules()?;
        let mut result = index.search_with_context(&query, filter, rules.as_ref())?;
        if hydrate {
            for hit in &mut result.hits {
                self.hydrate(uid, &mut hit.document)?;
                if let Some(formatted) = &mut hit.formatted {
                    self.hydrate(uid, formatted)?;
                }
                project(
                    &mut hit.document,
                    projection.as_ref(),
                    query.retrieve_vectors,
                );
                if let Some(formatted) = &mut hit.formatted {
                    project(formatted, projection.as_ref(), false);
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn resolve_filter(
        &self,
        uid: &str,
        value: Option<&Value>,
    ) -> Result<Option<IndexFilter>> {
        let Some(value) = value.filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let Some(filter) = Filter::from_json(value)? else {
            return Ok(None);
        };
        let features = self.get_experimental_features();
        if filter.use_contains_operator().is_some() && !features.contains_filter {
            return Err(Error::ExperimentalFeatureNotEnabled(
                "containsFilter".into(),
            ));
        }
        let index = self.get_index(uid)?;
        let txn = index.inner.read_txn()?;
        let keys = index.inner.foreign_keys(&txn)?;
        let mut used: HashMap<String, roaring::RoaringBitmap> = HashMap::new();
        IndexFilter::from_filter(filter, &mut |fid, condition| {
            if !features.foreign_keys {
                return Err(Error::ExperimentalFeatureNotEnabled("foreignKeys".into()));
            }
            let key = keys
                .iter()
                .find(|key| key.field_name == fid.fragment())
                .ok_or_else(|| {
                    Error::InvalidFilter(format!("{} is not a foreign key", fid.fragment()))
                })?;
            let foreign = self.get_index(&key.foreign_index_uid)?;
            let txn = foreign.inner.read_txn()?;
            let fields = foreign.inner.fields_ids_map(&txn)?;
            let local = IndexFilter::from_filter_without_foreign(Filter {
                condition: *condition,
            })
            .map_err(|_| Error::InvalidFilter("Nested foreign filters are not supported".into()))?;
            let ids = local.evaluate(&txn, &foreign.inner, &fields)?;
            let all = used.entry(key.foreign_index_uid.clone()).or_default();
            *all |= &ids;
            if all.len() > 1000 {
                return Err(Error::InvalidFilter(
                    "Foreign filters cannot retrieve more than 1000 documents per index".into(),
                ));
            }
            let els = foreign
                .inner
                .external_id_of(&txn, &fields, ids)?
                .into_iter()
                .map(|id| id.map(Into::into).map_err(Error::from))
                .collect::<Result<_>>()?;
            Ok(IndexFilterCondition::In { fid, els })
        })
        .map(Some)
    }

    pub(crate) fn hydrate(&self, uid: &str, document: &mut Value) -> Result<()> {
        let index = self.get_index(uid)?;
        let txn = index.inner.read_txn()?;
        for key in index.inner.foreign_keys(&txn)? {
            let foreign = self.get_index(&key.foreign_index_uid)?;
            let txn = foreign.inner.read_txn()?;
            let fields = foreign.inner.fields_ids_map(&txn)?;
            let mut cache = HashMap::new();
            visit_path(
                document,
                &key.field_name.split('.').collect::<Vec<_>>(),
                &mut |value| {
                    let Ok(id) = milli::documents::validate_document_id_value(value.clone()) else {
                        return Ok(());
                    };
                    if !cache.contains_key(&id) {
                        let doc = match foreign.inner.external_documents_ids().get(&txn, &id)? {
                            Some(id) => {
                                foreign.make_document(&txn, &fields, id, None, false, true)?
                            }
                            None => serde_json::json!({}),
                        };
                        cache.insert(id.clone(), doc);
                    }
                    *value = cache[&id].clone();
                    Ok(())
                },
            )?;
        }
        Ok(())
    }
}

fn project(document: &mut Value, fields: Option<&BTreeSet<String>>, retrieve_vectors: bool) {
    if let (Some(object), Some(fields)) = (
        document.as_object_mut(),
        fields.filter(|fields| !fields.contains("*")),
    ) {
        *object = permissive_json_pointer::select_values(
            std::mem::take(object),
            fields
                .iter()
                .map(String::as_str)
                .chain(retrieve_vectors.then_some("_vectors")),
        );
    }
}

fn visit_path(
    value: &mut Value,
    path: &[&str],
    f: &mut impl FnMut(&mut Value) -> Result<()>,
) -> Result<()> {
    if let Value::Array(values) = value {
        for value in values {
            visit_path(value, path, f)?;
        }
    } else if let Some((field, rest)) = path.split_first() {
        if let Some(value) = value.get_mut(*field) {
            visit_path(value, rest, f)?;
        }
    } else {
        f(value)?;
    }
    Ok(())
}

fn validate_threshold(threshold: Option<f64>) -> Result<()> {
    if threshold.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
        return Err(Error::Internal(
            "rankingScoreThreshold must be between 0 and 1".into(),
        ));
    }
    Ok(())
}
