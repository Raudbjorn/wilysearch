//! Local APIs that do not require server tasks or HTTP routes.
use super::Engine;
use crate::{core::Error, traits::Result, types::*};

impl Engine {
    pub fn get_search_rule(&self, uid: &RuleUid) -> Result<Option<DynamicSearchRule>> {
        self.inner.get_search_rule(uid)
    }
    pub fn update_search_rule(
        &self,
        uid: &RuleUid,
        update: DynamicSearchRuleUpdateRequest,
    ) -> Result<DynamicSearchRule> {
        self.inner.update_search_rule(uid, update)
    }
    pub fn delete_search_rule(&self, uid: &RuleUid) -> Result<bool> {
        self.inner.delete_search_rule(uid)
    }
    pub fn list_search_rules(&self, offset: usize, limit: usize) -> Result<Vec<DynamicSearchRule>> {
        self.inner.list_search_rules(offset, limit)
    }

    /// Rename an index. Foreign-key settings keep their explicit target UIDs.
    pub fn rename_index(&self, uid: &str, new_uid: &str) -> Result<TaskInfo> {
        self.inner.rename_index(uid, new_uid)?;
        self.mutation_task(new_uid, "indexUpdate")
    }

    pub fn update_documents_by_function(
        &self,
        uid: &str,
        request: &UpdateDocumentsByFunction,
    ) -> Result<TaskInfo> {
        if !self
            .inner
            .get_experimental_features()
            .edit_documents_by_function
        {
            return Err(Error::ExperimentalFeatureNotEnabled(
                "editDocumentsByFunction".into(),
            ));
        }
        let filter = self.inner.resolve_filter(uid, request.filter.as_ref())?;
        self.resolve_index(uid)?.update_by_function(
            filter,
            request.context.clone(),
            request.function.clone(),
        )?;
        self.mutation_task(uid, "documentEdition")
    }

    pub fn list_fields(&self, uid: &str, query: &FieldsQuery) -> Result<FieldsResponse> {
        use milli::{FieldSortOrder, PatternMatch};
        let index = self.resolve_index(uid)?;
        let txn = index.inner.read_txn()?;
        let fields = index.inner.fields_ids_map_with_metadata(&txn)?;
        let mut results = Vec::new();
        for (_, name, meta) in fields.iter() {
            let features = meta
                .filterable_attributes_features(fields.metadata_builder().filterable_attributes());
            let capabilities = [
                meta.displayed == PatternMatch::Match,
                meta.is_searchable() == PatternMatch::Match,
                meta.sortable == PatternMatch::Match,
                meta.distinct == PatternMatch::Match,
                meta.is_asc_desc() == PatternMatch::Match,
                features.facet_search || features.filter.equality || features.filter.comparison,
            ];
            if let Some(filter) = &query.filter {
                if filter
                    .attribute_patterns
                    .as_ref()
                    .is_some_and(|p| p.match_str(name) != PatternMatch::Match)
                {
                    continue;
                }
                let required = [
                    filter.displayed,
                    filter.searchable,
                    filter.sortable,
                    filter.distinct,
                    filter.ranking_rule,
                    filter.filterable,
                ];
                if required
                    .into_iter()
                    .zip(capabilities)
                    .any(|(required, actual)| required.is_some_and(|v| v != actual))
                {
                    continue;
                }
            }
            let locales = fields
                .metadata_builder()
                .localized_attributes_rules()
                .and_then(|r| meta.locales(r))
                .unwrap_or_default();
            results.push(serde_json::json!({
                "name": name, "displayed": {"enabled":capabilities[0]}, "searchable":{"enabled":capabilities[1]},
                "sortable":{"enabled":capabilities[2]}, "distinct":{"enabled":capabilities[3]},
                "rankingRule":{"enabled":capabilities[4],"order":meta.asc_desc.1.map(|o| match o { FieldSortOrder::Asc=>"asc", FieldSortOrder::Desc=>"desc" })},
                "filterable":{"enabled":capabilities[5], "sortBy": meilisearch_types::facet_values_sort::FacetValuesSort::from(meta.sort_by),
                    "facetSearch": features.facet_search, "equality":features.filter.equality, "comparison":features.filter.comparison},
                "localized":{"locales":locales}
            }));
        }
        results.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let total = results.len();
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(20);
        Ok(FieldsResponse {
            results: results.into_iter().skip(offset).take(limit).collect(),
            total,
            offset,
            limit,
        })
    }
}
