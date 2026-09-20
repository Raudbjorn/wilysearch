//! Settings use upstream validation and round-trip without lossy conversion.
use super::Engine;
use crate::{
    traits::{self, Result},
    types::*,
};

impl traits::SettingsApi for Engine {
    fn get_settings(&self, index_uid: &str) -> Result<Settings> {
        let mut settings = self.resolve_index(index_uid)?.get_settings()?;
        let flags = self.inner.get_experimental_features();
        if !flags.foreign_keys {
            settings.foreign_keys = Setting::NotSet;
        }
        if !flags.chat_completions {
            settings.chat = Setting::NotSet;
        }
        Ok(settings)
    }
    fn update_settings(&self, index_uid: &str, settings: &Settings) -> Result<TaskInfo> {
        let value = serde_json::to_value(settings)?;
        let flags = self.inner.get_experimental_features();
        for (name, enabled) in [
            ("foreignKeys", flags.foreign_keys),
            ("chat", flags.chat_completions),
        ] {
            if value.get(name).is_some_and(|v| !v.is_null()) && !enabled {
                return Err(crate::core::Error::ExperimentalFeatureNotEnabled(
                    name.into(),
                ));
            }
        }
        if let Some(embedders) = value.get("embedders").and_then(|v| v.as_object()) {
            for embedder in embedders.values() {
                if embedder.get("source").and_then(|v| v.as_str()) == Some("composite")
                    && !flags.composite_embedders
                {
                    return Err(crate::core::Error::ExperimentalFeatureNotEnabled(
                        "compositeEmbedders".into(),
                    ));
                }
                if ["indexingFragments", "searchFragments"]
                    .iter()
                    .any(|k| embedder.get(k).is_some_and(|v| !v.is_null()))
                    && !flags.multimodal
                {
                    return Err(crate::core::Error::ExperimentalFeatureNotEnabled(
                        "multimodal".into(),
                    ));
                }
            }
        }
        self.resolve_index(index_uid)?.update_settings(settings)?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
    fn reset_settings(&self, index_uid: &str) -> Result<TaskInfo> {
        self.resolve_index(index_uid)?.reset_settings()?;
        self.mutation_task(index_uid, "settingsUpdate")
    }
}
