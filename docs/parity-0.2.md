# Wilysearch 0.2 compatibility

The engine is pinned to Meilisearch **1.54.0 development**, revision
`1380adaacebad8a019d88549a06bae2dd90de249`. This is a development snapshot, not a stable Meilisearch release. Rust 1.98.1 is pinned in `rust-toolchain.toml`.

| Capability | Embedded API | Notes |
| --- | --- | --- |
| Keyword, vector, hybrid search | `Search::search` | Native milli ranking, thresholds, locales, distinct, negative operators, nested highlighting and match positions. `hybrid` selects a configured embedder. |
| Similar documents | `Search::similar` | Native vectors, filters, projections and vector retrieval. |
| Settings | `SettingsApi` | `Settings` is the upstream `Settings<Unchecked>` type. Preserves structured filterable rules, ranking rules, embedder fragments/composites, chat, foreign keys, facet/prefix search and cutoff settings. |
| Document browsing | `Documents` | Typed IDs, JSON filters, sort, nested field selection, vector retrieval. Browsing includes fields hidden from search. |
| Document mutation | `Documents` | Immediate replace/merge/delete; `skip_creation`; JSON input rejects CSV options. |
| Update by function | `Engine::update_documents_by_function` | Native bounded Rhai execution and transactional rollback. Runtime gate: `editDocumentsByFunction`. |
| Local federation | `Search::multi_search` | Native weighted ranking tuples, deduplication, distinct, merged facets, pin placement and `_federation` provenance. Uses up to `maxTotalHits` candidates per query in memory. Per-query pagination/facets/personalization are rejected in favor of federation options. |
| Foreign keys | Settings plus engine search/filter APIs | One-hop hydration and `_foreign` filters. Runtime gate: `foreignKeys`; nested foreign filters rejected; maximum 1,000 foreign matches per index per query. |
| Dynamic search rules | `Engine::{get,update,delete,list}_search_rule` | Persistent hidden rule index; native conditions, pin/scale actions and fuel limits. Runtime gate: `dynamicSearchRules`. |
| Template preview | `Engine::render_template` | Inline or stored document/chat templates; inline/indexing/search fragments. Runtime gates: `renderTemplates`, `multimodal` for fragments and `chatCompletions` for chat templates. |
| Index administration | `Indexes`, `Engine::rename_index`, `Engine::list_fields`, `System` | Rename, field capabilities, embedding counts, disk and used-size statistics. Release core index handles before rename. Foreign references retain their configured target UIDs. |
| Chat workspaces | `Engine::*_chat_workspace`, `ai::Chat` | Optional `ai` Cargo feature plus `chatCompletions` runtime gate. OpenAI, Azure OpenAI, Mistral and vLLM. Bounded retrieval tool loop, provider deltas, sources and caller-owned tool calls. |
| Personalization | `SearchRequest::personalize`, `FederationSettings::personalize` | Optional `ai` feature; configure `Engine::set_personalization` or `WilysearchConfig::personalization`. Synchronous Cohere client with retries, deadline fallback and score-preserving ordering. `ai::CohereReranker` also implements the async RAG trait. |
| External vector stores | Existing `core::VectorStore` and RAG APIs | Independent of native milli hybrid search; optional `surrealdb` backend remains available. |
| Backups | `System` / core maintenance APIs | Logical exports include settings, all document fields and vectors, rules and workspace configuration. Snapshots include format markers and index metadata. Each LMDB copy is consistent; simultaneous updates across different indexes do not form a single database-wide transaction. |

The low-level `core::Index` API operates on one index. Use `Engine` or `core::Meilisearch` for foreign lookups and dynamic rules. The async chat API uses blocking workers for local retrieval; the existing search traits stay synchronous.

Server-only features remain intentionally excluded: HTTP routes, authentication, task scheduling, batch queues, webhooks, server SSE/MCP, remote federation, sharding, replication and telemetry. Legacy traits for tasks, keys, batches and webhooks retain their existing empty/error behavior.

## Upgrade and rebuild

**0.1 databases cannot be opened by 0.2.** On startup, 0.2 checks `wilysearch-format.json` before opening LMDB or changing metadata. Unversioned nonempty storage, malformed manifests and mismatched revisions return `IncompatibleDatabase`. Do not add or edit this marker to bypass the check.

1. Keep the old database and an executable built with 0.1 until the rebuild is verified.
2. Export the original documents, vectors and index settings using the old application or the authoritative source data. The old dump implementation may omit hidden fields or vectors; verify completeness before relying on it.
3. Create a **new empty directory** with 0.2. Recreate indexes and primary keys, apply settings, then ingest the original documents. Enable experimental gates before applying foreign-key, composite, multimodal or chat settings. Automatic embedders may contact their configured providers during indexing.
4. Recreate any application-managed rules/workspaces. Run representative queries, compare counts and make a fresh snapshot.
5. Switch the application to the new directory. Roll back by using the old executable and old directory together; do not open new storage with 0.1.

Settings updates distinguish omission, reset and explicit values:

```rust
use wilysearch::types::{Settings, Setting};
let settings: Settings = serde_json::from_value(serde_json::json!({
    "filterableAttributes": ["genre"],
    "prefixSearch": "disabled",
    "searchCutoffMs": null
}))?;
// Unmentioned fields remain unchanged; null resets only that field.
// Rust callers may use Setting::Set, Setting::Reset, and Setting::NotSet.
```

`DocumentQuery.fields` and browsing IDs/filters/sort are now typed collections and JSON values. `SearchRequest.hybrid` is typed. Facet maps preserve native ordering. Bulk settings use the upstream types; old `Settings::new().with_*` builders are replaced by `Default`, `Setting` fields or JSON deserialization. Individual settings helpers remain available. As upstream does, `Engine::get_settings` omits chat and foreign-key settings while their runtime gates are disabled; backups preserve all settings.

The CLI accepts a complete request with `wily search INDEX --request search.json`; it conflicts with individual query flags. There are no dedicated chat or rules CLI commands.

## Provider configuration

Provider credentials are passed explicitly. Workspace reads return `[redacted]` for keys; workspace files contain credentials and are written with mode 0600 on Unix. Exports and snapshots also contain these files. Local/private provider URLs require `engine.allow_local_provider_urls = true`; this applies to native embedders, chat and Cohere.

```rust,no_run
use std::sync::Arc;
use wilysearch::ai::{Chat, CreateChatCompletionRequest};
// Enable chatCompletions and configure a workspace through Engine first.
# let engine: Arc<wilysearch::engine::Engine> = todo!();
let chat = Chat::new(engine, "assistant");
let request: CreateChatCompletionRequest = serde_json::from_value(serde_json::json!({
    "model": "your-provider-model",
    "messages": [{"role": "user", "content": "Find documents about Rust"}]
}))?;
let result = chat.complete(request).await?;
// result.messages includes tool results for continuing the conversation.
// chat.stream(request) yields ChatEvent::Chunk, Sources, ToolResult and Done.
```

Chat executes only `_meiliSearchInIndex`. Calls to user tools are returned to the caller; the library does not execute them. `Chat::indexes` restricts which local indexes the model can query. `max_tool_rounds` and `timeout` bound execution; dropping a stream cancels pending provider work. Tests use local mock servers and no real API keys.

## Validation matrix

```sh
cargo test --workspace
cargo test --workspace --features ai
cargo test --workspace --features surrealdb
cargo test --workspace --all-features
```

`tests/parity_tests.rs` covers the new local behavior and rebuild/snapshot contract. `tests/ai_tests.rs` exercises mocked provider routing, retrieval, live streaming, credentials and reranking. Existing CLI, settings, concurrency and external-vector tests are retained.
