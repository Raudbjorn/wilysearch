//! Stub trait implementations for features not supported in embedded mode.
//!
//! - `Tasks` -- no task queue
//! - `Batches` -- no batch processing
//! - `Keys` -- no authentication
//! - `Webhooks` -- no webhook delivery

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;

// ─── Tasks (stub -- no task queue in embedded mode) ──────────────────────────

impl traits::Tasks for Engine {
    fn get_task(&self, _task_uid: u64) -> Result<Task> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }

    fn list_tasks(&self, _filter: &TaskFilter) -> Result<TaskList> {
        Ok(TaskList {
            results: vec![],
            total: 0,
            limit: 20,
            from: None,
            next: None,
        })
    }

    fn cancel_tasks(&self, _filter: &TaskFilter) -> Result<TaskInfo> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }

    fn delete_tasks(&self, _filter: &TaskFilter) -> Result<TaskInfo> {
        Err(crate::core::error::Error::Internal("Tasks are not supported in embedded mode".to_string()))
    }
}

// ─── Batches (stub) ──────────────────────────────────────────────────────────

impl traits::Batches for Engine {
    fn get_batch(&self, _batch_uid: u64) -> Result<Batch> {
        Err(crate::core::error::Error::Internal("Batches are not supported in embedded mode".to_string()))
    }

    fn list_batches(&self, _filter: &TaskFilter) -> Result<BatchList> {
        Ok(BatchList {
            results: vec![],
            total: 0,
            limit: 20,
            from: None,
            next: None,
        })
    }
}

// ─── Keys (stub -- no auth in embedded mode) ─────────────────────────────────

impl traits::Keys for Engine {
    fn list_keys(&self, _query: &PaginationQuery) -> Result<ApiKeyList> {
        Ok(ApiKeyList {
            results: vec![],
            offset: 0,
            limit: 20,
            total: 0,
        })
    }
    fn get_key(&self, _key_id: &str) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn create_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn update_key(&self, _key_id: &str, _request: &UpdateApiKeyRequest) -> Result<ApiKey> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
    fn delete_key(&self, _key_id: &str) -> Result<()> {
        Err(crate::core::error::Error::Internal("API keys are not supported in embedded mode".to_string()))
    }
}

// ─── Webhooks (stub -- no webhooks in embedded mode) ─────────────────────────

impl traits::Webhooks for Engine {
    fn list_webhooks(&self) -> Result<Vec<Webhook>> {
        Ok(vec![])
    }
    fn get_webhook(&self, _webhook_uid: &str) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn create_webhook(&self, _request: &CreateWebhookRequest) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn update_webhook(&self, _uid: &str, _request: &UpdateWebhookRequest) -> Result<Webhook> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
    fn delete_webhook(&self, _webhook_uid: &str) -> Result<()> {
        Err(crate::core::error::Error::Internal("Webhooks are not supported in embedded mode".to_string()))
    }
}
