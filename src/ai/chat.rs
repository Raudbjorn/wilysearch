use super::{ChatCompletionSource, WorkspaceSettings};
use crate::{
    core::{Error, Result},
    engine::Engine,
    types::SearchResponse,
};
use async_openai::{
    Client,
    config::{AzureConfig, OpenAIConfig},
    types::*,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const SEARCH_TOOL: &str = "_meiliSearchInIndex";

#[derive(Clone)]
pub struct Chat {
    engine: Arc<Engine>,
    workspace: String,
    /// Restrict retrieval to these local indexes. None exposes all user indexes.
    pub indexes: Option<Vec<String>>,
    pub max_tool_rounds: usize,
    /// Total provider and retrieval timeout, including all tool rounds.
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSource {
    pub index_uid: String,
    pub results: SearchResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResult {
    pub response: CreateChatCompletionResponse,
    pub sources: Vec<ChatSource>,
    /// Includes internal tool results; append results for caller-owned tools to continue.
    pub messages: Vec<ChatCompletionRequestMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    Chunk {
        chunk: CreateChatCompletionStreamResponse,
    },
    Sources {
        source: ChatSource,
    },
    ToolResult {
        id: String,
        content: String,
    },
    Done {
        messages: Vec<ChatCompletionRequestMessage>,
    },
}

impl Chat {
    pub fn new(engine: Arc<Engine>, workspace: impl Into<String>) -> Self {
        Self {
            engine,
            workspace: workspace.into(),
            indexes: None,
            max_tool_rounds: 8,
            timeout: Duration::from_secs(120),
        }
    }

    pub async fn complete(&self, request: CreateChatCompletionRequest) -> Result<ChatResult> {
        tokio::time::timeout(self.timeout, self.complete_inner(request))
            .await
            .map_err(|_| Error::Internal("Chat deadline exceeded".into()))?
    }

    async fn complete_inner(&self, mut request: CreateChatCompletionRequest) -> Result<ChatResult> {
        let (provider, indexes) = self.prepare(&mut request).await?;
        request.stream = Some(false);
        let mut sources = Vec::new();
        for round in 0..=self.max_tool_rounds {
            let response = provider.complete(request.clone()).await?;
            let choice = response
                .choices
                .first()
                .ok_or_else(|| Error::Internal("Provider returned no choices".into()))?;
            request
                .messages
                .push(serde_json::from_value(serde_json::to_value(
                    &choice.message,
                )?)?);
            let calls = choice.message.tool_calls.as_deref().unwrap_or_default();
            let internal = calls.iter().any(|c| c.function.name == SEARCH_TOOL);
            if internal && round == self.max_tool_rounds {
                return Err(Error::Internal("Chat tool round limit exceeded".into()));
            }
            for call in calls.iter().filter(|c| c.function.name == SEARCH_TOOL) {
                let (source, content) = self.retrieve(&call.function.arguments, &indexes).await?;
                if let Some(source) = source {
                    sources.push(source);
                }
                request.messages.push(tool_message(&call.id, &content)?);
            }
            if !internal || calls.iter().any(|c| c.function.name != SEARCH_TOOL) {
                return Ok(ChatResult {
                    response,
                    sources,
                    messages: request.messages,
                });
            }
        }
        unreachable!()
    }

    /// Forward provider deltas as they arrive, including across retrieval rounds.
    /// Dropping the stream cancels pending provider requests.
    pub fn stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> ReceiverStream<Result<ChatEvent>> {
        let (tx, rx) = mpsc::channel(32);
        let chat = self.clone();
        tokio::spawn(async move {
            let result = tokio::select! {
                _ = tx.closed() => return,
                result = tokio::time::timeout(chat.timeout, chat.stream_inner(request, &tx)) =>
                    result.unwrap_or_else(|_| Err(Error::Internal("Chat deadline exceeded".into()))),
            };
            if let Err(error) = result {
                let _ = tx.send(Err(error)).await;
            }
        });
        ReceiverStream::new(rx)
    }

    async fn stream_inner(
        &self,
        mut request: CreateChatCompletionRequest,
        tx: &mpsc::Sender<Result<ChatEvent>>,
    ) -> Result<()> {
        let (provider, indexes) = self.prepare(&mut request).await?;
        request.stream = Some(true);
        for round in 0..=self.max_tool_rounds {
            let mut stream = provider.stream(request.clone()).await?;
            let mut calls: BTreeMap<u32, (String, String, String)> = BTreeMap::new();
            let mut content = String::new();
            let mut refusal = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(provider_error)?;
                for choice in &chunk.choices {
                    if choice.index != 0 {
                        return Err(Error::Internal(
                            "Chat supports one completion choice".into(),
                        ));
                    }
                    if let Some(text) = &choice.delta.content {
                        content.push_str(text);
                    }
                    if let Some(text) = &choice.delta.refusal {
                        refusal.push_str(text);
                    }
                    for call in choice.delta.tool_calls.as_deref().unwrap_or_default() {
                        let entry = calls.entry(call.index).or_default();
                        if let Some(id) = &call.id {
                            entry.0.push_str(id);
                        }
                        if let Some(function) = &call.function {
                            if let Some(name) = &function.name {
                                entry.1.push_str(name);
                            }
                            if let Some(args) = &function.arguments {
                                entry.2.push_str(args);
                            }
                        }
                    }
                }
                send(tx, ChatEvent::Chunk { chunk }).await?;
            }
            let calls: Vec<Value> = calls.into_values().map(|(id,name,arguments)| json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments}})).collect();
            let internal = calls.iter().any(|c| c["function"]["name"] == SEARCH_TOOL);
            request.messages.push(serde_json::from_value(
                json!({"role":"assistant", "content":content,
                "refusal":if refusal.is_empty(){Value::Null}else{Value::String(refusal)},
                "tool_calls":if calls.is_empty(){Value::Null}else{json!(calls)}}),
            )?);
            if internal && round == self.max_tool_rounds {
                return Err(Error::Internal("Chat tool round limit exceeded".into()));
            }
            for call in calls
                .iter()
                .filter(|c| c["function"]["name"] == SEARCH_TOOL)
            {
                let id = call["id"]
                    .as_str()
                    .ok_or_else(|| Error::Internal("Tool call missing ID".into()))?;
                let (source, content) = self
                    .retrieve(
                        call["function"]["arguments"].as_str().unwrap_or(""),
                        &indexes,
                    )
                    .await?;
                request.messages.push(tool_message(id, &content)?);
                if let Some(source) = source {
                    send(tx, ChatEvent::Sources { source }).await?;
                }
                send(
                    tx,
                    ChatEvent::ToolResult {
                        id: id.into(),
                        content,
                    },
                )
                .await?;
            }
            if !internal || calls.iter().any(|c| c["function"]["name"] != SEARCH_TOOL) {
                return send(
                    tx,
                    ChatEvent::Done {
                        messages: request.messages,
                    },
                )
                .await;
            }
        }
        unreachable!()
    }

    async fn prepare(
        &self,
        request: &mut CreateChatCompletionRequest,
    ) -> Result<(Provider, Vec<String>)> {
        self.engine.require_chat()?;
        if request.n.is_some_and(|n| n != 1) {
            return Err(Error::Internal(
                "Chat supports one completion choice".into(),
            ));
        }
        if request.messages.is_empty() || request.model.trim().is_empty() {
            return Err(Error::Internal("Chat requires a model and messages".into()));
        }
        let engine = self.engine.clone();
        let workspace = self.workspace.clone();
        let indexes = self.indexes.clone();
        let (settings, indexes, descriptions) =
            tokio::task::spawn_blocking(move || -> Result<_> {
                let settings = engine.workspaces()?.remove(&workspace).ok_or_else(|| {
                    Error::Internal(format!("Chat workspace not found: {workspace}"))
                })?;
                let indexes = indexes
                    .map(Ok)
                    .unwrap_or_else(|| engine.inner.list_indexes())?;
                let mut descriptions = String::new();
                for uid in &indexes {
                    let index = engine.inner.get_index(uid)?;
                    let txn = index.inner.read_txn()?;
                    descriptions.push_str(&format!(
                        "\n{uid}: {}",
                        index.inner.chat_config(&txn)?.description
                    ));
                }
                Ok((settings, indexes, descriptions))
            })
            .await
            .map_err(provider_error)??;
        let role = match settings.source.system_role(&request.model) {
            meilisearch_types::features::SystemRole::System => "system",
            meilisearch_types::features::SystemRole::Developer => "developer",
        };
        request.messages.insert(
            0,
            serde_json::from_value(json!({"role":role,"content":settings.prompts.system}))?,
        );
        let tools = request.tools.get_or_insert_with(Vec::new);
        if tools.iter().any(|t| t.function.name == SEARCH_TOOL) {
            return Err(Error::Internal("Reserved search tool name".into()));
        }
        tools.push(serde_json::from_value(json!({"type":"function", "function":{
            "name":SEARCH_TOOL, "description":format!("{}{}", settings.prompts.search_description, descriptions),
            "parameters":{"type":"object", "properties":{
                "index_uid":{"type":"string","enum":indexes,"description":settings.prompts.search_index_uid_param},
                "q":{"type":"string","description":settings.prompts.search_q_param},
                "filter":{"type":"string","description":settings.prompts.search_filter_param}
            }, "required":["index_uid","q","filter"],"additionalProperties":false}
        }}))?);
        Ok((
            Provider::new(&settings, self.engine.inner.options.ip_policy())?,
            indexes,
        ))
    }

    async fn retrieve(
        &self,
        arguments: &str,
        indexes: &[String],
    ) -> Result<(Option<ChatSource>, String)> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            index_uid: String,
            q: Option<String>,
            filter: Option<Value>,
        }
        let args: Arguments = match serde_json::from_str(arguments) {
            Ok(args) => args,
            Err(error) => return Ok(tool_error(error)),
        };
        if !indexes.contains(&args.index_uid) {
            return Ok(tool_error("Search tool selected an unavailable index"));
        }
        let engine = self.engine.clone();
        tokio::task::spawn_blocking(move || -> Result<_> {
            let index = engine.inner.get_index(&args.index_uid)?;
            let txn = index.inner.read_txn()?;
            let config = index.inner.chat_config(&txn)?;
            let mut request: crate::types::SearchRequest =
                serde_json::from_value(serde_json::to_value(config.search_parameters)?)?;
            request.q = args.q;
            request.filter = args
                .filter
                .filter(|f| !f.is_null() && f.as_str() != Some(""));
            let result = match crate::traits::Search::search(&*engine, &args.index_uid, &request) {
                Ok(result) => result,
                Err(error) => return Ok(tool_error(error)),
            };
            let prompt: milli::prompt::Prompt = config.prompt.try_into().map_err(provider_error)?;
            let metadata = RwLock::new(index.inner.fields_ids_map_with_metadata(&txn)?);
            let global = std::cell::RefCell::new(milli::GlobalFieldsIdsMap::new(&metadata));
            let alloc = bumpalo::Bump::new();
            let mut text = Vec::new();
            for (position, hit) in result.hits.iter().enumerate() {
                let raw = alloc.alloc_str(&serde_json::to_string(hit)?);
                let raw: &serde_json::value::RawValue = serde_json::from_str(raw)?;
                let doc = bumparaw_collections::RawMap::from_raw_value(raw, &alloc)?;
                let rendered = prompt
                    .render_document(None, &doc, &global, &alloc)
                    .map_err(provider_error)?;
                text.push(format!("[{}:{}] {rendered}", args.index_uid, position + 1));
            }
            Ok((
                Some(ChatSource {
                    index_uid: args.index_uid,
                    results: result,
                }),
                text.join("\n"),
            ))
        })
        .await
        .map_err(provider_error)?
    }
}
use std::sync::RwLock;
fn tool_error(error: impl std::fmt::Display) -> (Option<ChatSource>, String) {
    (
        None,
        format!("Search failed: {error}. Correct the tool arguments and retry."),
    )
}

fn tool_message(id: &str, content: &str) -> Result<ChatCompletionRequestMessage> {
    Ok(serde_json::from_value(
        json!({"role":"tool","tool_call_id":id,"content":content}),
    )?)
}
async fn send(tx: &mpsc::Sender<Result<ChatEvent>>, event: ChatEvent) -> Result<()> {
    tx.send(Ok(event))
        .await
        .map_err(|_| Error::Internal("Chat stream closed".into()))
}
fn provider_error(error: impl std::fmt::Display) -> Error {
    Error::Internal(error.to_string())
}

pub(super) fn validate_settings(settings: &WorkspaceSettings) -> Result<()> {
    for value in [&settings.api_key, &settings.org_id, &settings.project_id] {
        if value
            .as_ref()
            .is_some_and(|s| !s.is_ascii() || s.chars().any(char::is_control))
        {
            return Err(Error::Internal("Invalid provider header value".into()));
        }
    }
    if matches!(
        settings.source,
        ChatCompletionSource::VLlm | ChatCompletionSource::AzureOpenAi
    ) && settings.base_url.is_none()
    {
        return Err(Error::Internal("This provider requires baseUrl".into()));
    }
    if let Some(url) = &settings.base_url {
        let url = http_client::reqwest::Url::parse(url).map_err(provider_error)?;
        if !["http", "https"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Internal("Invalid provider URL".into()));
        }
    }
    if settings.source == ChatCompletionSource::AzureOpenAi
        && (settings.deployment_id.is_none() || settings.api_version.is_none())
    {
        return Err(Error::Internal(
            "Azure requires deploymentId and apiVersion".into(),
        ));
    }
    Ok(())
}

enum Provider {
    OpenAi(Client<OpenAIConfig>),
    Azure(Client<AzureConfig>),
}
impl Provider {
    fn new(settings: &WorkspaceSettings, policy: http_client::policy::IpPolicy) -> Result<Self> {
        validate_settings(settings)?;
        let key = settings.api_key.as_deref().unwrap_or("");
        Ok(if settings.source == ChatCompletionSource::AzureOpenAi {
            Self::Azure(Client::with_config(
                policy,
                AzureConfig::new()
                    .with_api_key(key)
                    .with_api_base(
                        settings
                            .base_url
                            .as_deref()
                            .unwrap_or_default()
                            .trim_end_matches('/'),
                    )
                    .with_deployment_id(settings.deployment_id.as_deref().unwrap_or_default())
                    .with_api_version(settings.api_version.as_deref().unwrap_or_default()),
            ))
        } else {
            let config = OpenAIConfig::new()
                .with_api_key(key)
                .with_org_id(settings.org_id.as_deref().unwrap_or_default())
                .with_project_id(settings.project_id.as_deref().unwrap_or_default())
                .with_api_base(
                    settings
                        .base_url
                        .as_deref()
                        .or(settings.source.base_url())
                        .unwrap_or_default()
                        .trim_end_matches('/'),
                );
            Self::OpenAi(Client::with_config(policy, config))
        })
    }
    async fn complete(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse> {
        match self {
            Self::OpenAi(c) => c.chat().create(request).await,
            Self::Azure(c) => c.chat().create(request).await,
        }
        .map_err(provider_error)
    }
    async fn stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<ChatCompletionResponseStream> {
        match self {
            Self::OpenAi(c) => c.chat().create_stream(request).await,
            Self::Azure(c) => c.chat().create_stream(request).await,
        }
        .map_err(provider_error)
    }
}
