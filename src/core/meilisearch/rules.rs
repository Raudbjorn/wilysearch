use std::sync::Arc;

use super::Meilisearch;
use crate::core::{Error, Index, Result};
pub use meilisearch_types::dynamic_search_rules::{
    DynamicSearchRule, DynamicSearchRuleUpdateRequest, RuleUid,
};
use milli::dynamic_search_rules::{DsrFuel, DynamicSearchRules};

pub(crate) fn fuel() -> DsrFuel {
    DsrFuel::new(
        10,
        1000,
        100,
        10,
        4096,
        4096,
        10,
        milli::FilterConstraintFuel::new(100, 100, 25),
    )
}

impl Meilisearch {
    pub(crate) fn rule_index(&self, create: bool) -> Result<Option<Arc<Index>>> {
        let mut cache = self
            .rules
            .lock()
            .map_err(|_| Error::Internal("Rule index lock poisoned".into()))?;
        if let Some(index) = &*cache {
            return Ok(Some(index.clone()));
        }
        let path = self.options.db_path.join("internal").join("rules");
        let exists = path.join("data.mdb").exists();
        if !exists && !create {
            return Ok(None);
        }
        std::fs::create_dir_all(&path)?;
        let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
        options.map_size(self.options.max_index_size);
        let inner = milli::Index::new(
            options,
            path,
            if exists {
                milli::CreateOrOpen::Open
            } else {
                milli::CreateOrOpen::create_without_shards()
            },
        )?;
        let index = Arc::new(Index::new(inner, None));
        if !exists {
            index.update_primary_key("uid")?;
            index.update_settings(&serde_json::from_value(serde_json::json!({
                "searchableAttributes": ["conditions.query.words", "description"],
                "filterableAttributes": ["active", "conditions.time.start", "conditions.time.end", "conditions.query.isEmpty", "conditions.filter.values.*", "conditions.filter.nbConstraints"],
                "sortableAttributes": ["precedence", "lastUpdatedAt"],
                "proximityPrecision": "byAttribute", "typoTolerance": {"enabled": false, "disableOnNumbers": true},
                "facetSearch": false, "prefixSearch": "disabled"
            }))?)?;
            let mut txn = index.inner.write_txn()?;
            let fields = index.inner.fields_ids_map(&txn)?;
            milli::dynamic_search_rules::create_metadata(
                (1, 54, 0),
                &index.inner,
                &mut txn,
                &fields,
                &milli::progress::Progress::quiet(),
                &milli::update::IndexerConfig::default(),
                &milli::MustStopProcessing::default(),
                &http_client::policy::IpPolicy::deny_all_local_ips(),
            )?;
            txn.commit()?;
        }
        *cache = Some(index.clone());
        Ok(Some(index))
    }

    pub(crate) fn search_rules(&self) -> Result<Option<DynamicSearchRules>> {
        if !self.get_experimental_features().dynamic_search_rules {
            return Ok(None);
        }
        self.rule_index(false)?
            .map(|i| DynamicSearchRules::new(i.inner.clone()).map_err(Error::from))
            .transpose()
    }

    fn require_rules(&self) -> Result<()> {
        if !self.get_experimental_features().dynamic_search_rules {
            return Err(Error::ExperimentalFeatureNotEnabled(
                "dynamicSearchRules".into(),
            ));
        }
        Ok(())
    }

    pub fn get_search_rule(&self, uid: &RuleUid) -> Result<Option<DynamicSearchRule>> {
        self.require_rules()?;
        let Some(rules) = self.search_rules()? else {
            return Ok(None);
        };
        rules
            .get(uid.as_str())?
            .map(|doc| {
                DynamicSearchRule::try_from_meili_doc(doc, milli::FaultSource::Runtime)
                    .map_err(Error::from)
            })
            .transpose()
    }

    pub fn update_search_rule(
        &self,
        uid: &RuleUid,
        update: DynamicSearchRuleUpdateRequest,
    ) -> Result<DynamicSearchRule> {
        self.require_rules()?;
        let _guard = self
            .rules_write
            .lock()
            .map_err(|_| Error::Internal("Rule update lock poisoned".into()))?;
        let index = self.rule_index(true)?.expect("created rule index");
        let mut rule = self
            .get_search_rule(uid)?
            .unwrap_or_else(|| DynamicSearchRule::new(uid.clone()));
        rule.apply_update(update, time::OffsetDateTime::now_utc());
        let count = rule.facet_count();
        let mut document = serde_json::to_value(&rule)?;
        if let Some(serde_json::Value::Object(filter)) = document.pointer_mut("/conditions/filter")
        {
            filter.insert("nbConstraints".into(), count.into());
        }
        index.add_documents(vec![document], Some("uid"))?;
        Ok(rule)
    }

    pub fn delete_search_rule(&self, uid: &RuleUid) -> Result<bool> {
        self.require_rules()?;
        let _guard = self
            .rules_write
            .lock()
            .map_err(|_| Error::Internal("Rule update lock poisoned".into()))?;
        match self.rule_index(false)? {
            Some(index) => index.delete_document(uid.as_str()),
            None => Ok(false),
        }
    }

    pub fn list_search_rules(&self, offset: usize, limit: usize) -> Result<Vec<DynamicSearchRule>> {
        self.require_rules()?;
        let Some(rules) = self.search_rules()? else {
            return Ok(Vec::new());
        };
        rules
            .all_rule_ids()?
            .iter()
            .skip(offset)
            .take(limit)
            .map(|id| {
                let doc = rules
                    .get_from_internal_id(id)?
                    .ok_or_else(|| Error::Internal("Missing rule document".into()))?;
                Ok(DynamicSearchRule::try_from_meili_doc(
                    doc,
                    milli::FaultSource::Runtime,
                )?)
            })
            .collect()
    }
}
