//! Optional asynchronous chat retrieval and synchronous Cohere personalization.
//! Provider credentials are supplied explicitly; workspace reads redact them.
mod chat;
mod cohere;
pub use async_openai::types;
pub use async_openai::types::{
    CreateChatCompletionRequest, CreateChatCompletionResponse, CreateChatCompletionStreamResponse,
};
pub use chat::{Chat, ChatEvent, ChatResult, ChatSource};
pub use cohere::{CohereConfig, CohereReranker};
pub use meilisearch_types::features::{
    ChatCompletionSettings as WorkspaceSettings, ChatCompletionSource,
};

use crate::{
    core::{Error, Result},
    engine::Engine,
};
use std::collections::BTreeMap;

impl Engine {
    fn workspaces(&self) -> Result<BTreeMap<String, WorkspaceSettings>> {
        match std::fs::read(self.inner.options.db_path.join("chat-workspaces.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn list_chat_workspaces(&self) -> Result<Vec<String>> {
        Ok(self.workspaces()?.into_keys().collect())
    }
    pub fn get_chat_workspace(&self, uid: &str) -> Result<Option<WorkspaceSettings>> {
        Ok(self.workspaces()?.remove(uid).map(|mut settings| {
            settings.api_key = settings.api_key.map(|_| "[redacted]".into());
            settings
        }))
    }
    /// Replace a workspace, including its credentials. Credentials are never read from environment variables.
    pub fn set_chat_workspace(&self, uid: &str, settings: WorkspaceSettings) -> Result<()> {
        self.require_chat()?;
        if uid.is_empty() || uid.len() > 512 {
            return Err(Error::Internal("Invalid workspace UID".into()));
        }
        chat::validate_settings(&settings)?;
        let _guard = self
            .workspace_lock
            .lock()
            .map_err(|_| Error::Internal("Workspace lock poisoned".into()))?;
        let mut workspaces = self.workspaces()?;
        workspaces.insert(uid.into(), settings);
        self.save_workspaces(&workspaces)
    }
    pub fn delete_chat_workspace(&self, uid: &str) -> Result<bool> {
        self.require_chat()?;
        let _guard = self
            .workspace_lock
            .lock()
            .map_err(|_| Error::Internal("Workspace lock poisoned".into()))?;
        let mut workspaces = self.workspaces()?;
        let removed = workspaces.remove(uid).is_some();
        if removed {
            self.save_workspaces(&workspaces)?;
        }
        Ok(removed)
    }
    fn save_workspaces(&self, workspaces: &BTreeMap<String, WorkspaceSettings>) -> Result<()> {
        use std::io::Write;
        let path = self.inner.options.db_path.join("chat-workspaces.json");
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(&serde_json::to_vec_pretty(workspaces)?)?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temp);
        result
    }
    pub(crate) fn require_chat(&self) -> Result<()> {
        if !self.inner.get_experimental_features().chat_completions {
            return Err(Error::ExperimentalFeatureNotEnabled(
                "chatCompletions".into(),
            ));
        }
        Ok(())
    }
    pub fn set_personalization(&self, config: Option<CohereConfig>) -> Result<()> {
        let reranker = config
            .map(|c| CohereReranker::new(c, self.inner.options.allow_local_provider_urls))
            .transpose()?;
        *self
            .personalization
            .write()
            .map_err(|_| Error::Internal("Personalization lock poisoned".into()))? = reranker;
        Ok(())
    }
}
