use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CohereConfig {
    pub api_key: String,
    pub url: String,
    pub model: String,
    pub timeout_ms: u64,
}
impl Default for CohereConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            url: "https://api.cohere.ai/v1/rerank".into(),
            model: "rerank-english-v3.0".into(),
            timeout_ms: 30_000,
        }
    }
}
impl std::fmt::Debug for CohereConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CohereConfig")
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct CohereReranker {
    config: CohereConfig,
    policy: http_client::policy::IpPolicy,
}
impl CohereReranker {
    pub fn new(config: CohereConfig, allow_local_urls: bool) -> Result<Self> {
        if config.api_key.trim().is_empty()
            || !config.api_key.is_ascii()
            || config.api_key.chars().any(char::is_control)
        {
            return Err(Error::Internal("Cohere requires a valid API key".into()));
        }
        let url = http_client::reqwest::Url::parse(&config.url)
            .map_err(|_| Error::Internal("Invalid Cohere URL".into()))?;
        if !["http", "https"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Internal("Invalid Cohere URL".into()));
        }
        if config.timeout_ms == 0 || config.model.is_empty() {
            return Err(Error::Internal(
                "Cohere requires a model and a positive timeout".into(),
            ));
        }
        let policy = if allow_local_urls {
            http_client::policy::IpPolicy::danger_always_allow()
        } else {
            http_client::policy::IpPolicy::deny_all_local_ips()
        };
        policy
            .check_ip_in_hostname(&url)
            .map_err(|_| Error::Internal("Cohere URL blocked by IP policy".into()))?;
        Ok(Self { config, policy })
    }

    /// Return an ordering; scores and document contents remain untouched. Deadline expiry preserves the original order.
    pub fn order(
        &self,
        query: Option<&str>,
        user_context: &str,
        documents: &[Value],
        deadline: Instant,
    ) -> Result<Vec<usize>> {
        let original = || (0..documents.len()).collect();
        if documents.is_empty() || Instant::now() >= deadline {
            return Ok(original());
        }
        let prompt = match query {
            Some(q) => format!("User Context: {user_context}\nQuery: {q}"),
            None => format!("User Context: {user_context}"),
        };
        let documents_text = documents
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let body =
            json!({"model": self.config.model, "query": prompt, "documents": documents_text});
        for attempt in 0..=10 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(original());
            }
            let timeout = remaining.min(Duration::from_millis(self.config.timeout_ms));
            let agent = http_client::ureq::config::Config::builder()
                .prepare(|c| c.timeout_global(Some(timeout)).http_status_as_error(false))
                .build()
                .new_agent(self.policy.clone());
            let response = agent
                .post(&self.config.url)
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .send_json(&body);
            match response {
                Ok(mut response) if response.status().is_success() => {
                    #[derive(Deserialize)]
                    struct Response {
                        results: Vec<Hit>,
                    }
                    #[derive(Deserialize)]
                    struct Hit {
                        index: usize,
                    }
                    let response: Response = response
                        .body_mut()
                        .read_json()
                        .map_err(|_| Error::Internal("Invalid Cohere response".into()))?;
                    let mut seen = HashSet::new();
                    let order: Vec<_> = response.results.into_iter().map(|h| h.index).collect();
                    if order.len() != documents.len()
                        || order
                            .iter()
                            .any(|&i| i >= documents.len() || !seen.insert(i))
                    {
                        return Err(Error::Internal(
                            "Cohere response must contain each document exactly once".into(),
                        ));
                    }
                    return Ok(order);
                }
                Ok(response)
                    if response.status().as_u16() == 401 || response.status().as_u16() == 403 =>
                {
                    return Err(Error::Internal("Cohere authentication failed".into()));
                }
                Ok(response)
                    if response.status().as_u16() != 429
                        && !response.status().is_server_error() =>
                {
                    return Err(Error::Internal(format!(
                        "Cohere request failed: HTTP {}",
                        response.status().as_u16()
                    )));
                }
                _ if Instant::now() >= deadline => return Ok(original()),
                _ if attempt == 10 => {
                    return Err(Error::Internal(
                        "Cohere request failed after retries".into(),
                    ));
                }
                _ => std::thread::sleep(
                    Duration::from_millis(10 * 2u64.pow(attempt))
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
            }
        }
        unreachable!()
    }
}

impl crate::core::rag::Reranker for CohereReranker {
    type Document = Value;
    async fn rerank(
        &self,
        query: &str,
        results: Vec<crate::core::rag::RetrievalResult<Value>>,
        top_k: usize,
    ) -> Result<Vec<crate::core::rag::RetrievalResult<Value>>> {
        let reranker = self.clone();
        let query = query.to_owned();
        tokio::task::spawn_blocking(move || {
            let docs = results
                .iter()
                .map(|r| r.document.clone())
                .collect::<Vec<_>>();
            let order = reranker.order(
                Some(&query),
                "",
                &docs,
                Instant::now() + Duration::from_millis(reranker.config.timeout_ms),
            )?;
            Ok(order
                .into_iter()
                .take(top_k)
                .map(|i| results[i].clone())
                .collect())
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
    }
}
