# Tool Execution Architecture for wilysearch

## Phase 1: Requirements

### Context

The `wilysearch` crate provides an embedded Rust API wrapping milli (the Meilisearch indexing engine). This document specifies a future tool/function calling architecture for LLM integration.

### User Stories

**US-1: As a Rust developer embedding Meilisearch, I want to define custom tools so that the LLM can execute actions on my behalf.**

**US-2: As a library consumer, I want the tool execution loop handled automatically so that I don't need to manually orchestrate LLM-tool interactions.**

**US-3: As a developer, I want provider-agnostic tool definitions so that I can switch between OpenAI, Anthropic, and other providers without rewriting tool logic.**

### Acceptance Criteria (EARS Format)

#### Tool Definition
1. WHEN user defines a tool using `Tool` struct THEN system SHALL accept name, description, and JSON schema for parameters
2. WHEN user defines a tool function THEN system SHALL validate JSON schema at compile time (via serde) or runtime
3. IF tool schema is invalid THEN system SHALL return `Error::InvalidToolSchema` with details

#### Tool Registration
4. WHEN user calls `chat_completion_with_tools` THEN system SHALL accept `Vec<Tool>` parameter
5. WHEN tools are registered THEN system SHALL convert them to provider-specific format (OpenAI/Anthropic)
6. IF provider does not support tools THEN system SHALL return `Error::ToolsNotSupported`

#### Tool Execution Loop
7. WHEN LLM returns `finish_reason: tool_calls` THEN system SHALL parse tool calls from response
8. WHEN tool call is parsed THEN system SHALL invoke user-provided executor function
9. WHEN tool execution completes THEN system SHALL format result and continue conversation
10. WHEN tool execution fails THEN system SHALL format error as tool result (not abort)
11. WHEN LLM returns `finish_reason: stop` THEN system SHALL break loop and return final response

#### Streaming
12. WHEN streaming with tools THEN system SHALL emit chunks for both text and tool call accumulation
13. WHEN tool call JSON spans multiple chunks THEN system SHALL buffer and parse complete JSON
14. WHEN streaming tool execution THEN system SHALL emit `ChatChunk` with tool call metadata

#### Provider Compatibility
15. WHEN provider is OpenAI THEN system SHALL use `tools` parameter with `function` type
16. WHEN provider is Anthropic THEN system SHALL use `tools` parameter with `input_schema`
17. WHEN provider is Azure/Mistral/vLLM THEN system SHALL use OpenAI-compatible format

### Edge Cases

- **Parallel tool calls**: LLM may request multiple tools in single response
- **Recursive tool use**: Tool result may trigger additional tool calls (max depth needed)
- **Malformed tool JSON**: Graceful handling of invalid JSON in tool arguments
- **Tool timeout**: Long-running tools should have configurable timeout
- **Context overflow**: Tool results may exceed context window

---

## Phase 2: Design

### Decision: Library Selection

**Context:** Need to choose between building custom tool abstraction vs. using existing Rust crates.

**Options Considered:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Custom implementation** | Full control, minimal deps, matches existing patterns | More code to maintain, reinvent wheel |
| **B. `rig-rs`** | Best DX, `#[tool]` macro, agent abstraction | Heavy dependency, may conflict with our architecture |
| **C. `rllm`** | Standardized `ToolCall` structs, modular | Less mature, may need adaptation |
| **D. `async-openai` types only** | Already using it, mature types | No Anthropic native support |

**Decision:** **Option A (Custom implementation)** with inspiration from `rig-rs` patterns.

**Rationale:**
1. HTTP routes already have working tool call handling—we can extract and reuse
2. Existing `async-openai` types provide solid foundation
3. Minimal new dependencies aligns with Meilisearch philosophy
4. Full control over streaming behavior and error handling

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Wilysearch                          │
├─────────────────────────────────────────────────────────────┤
│  chat_completion_with_tools(&self, request, tools, executor)│
│                            │                                │
│  ┌─────────────────────────▼─────────────────────────────┐  │
│  │              Tool Execution Loop                       │  │
│  │  ┌──────────┐    ┌──────────┐    ┌──────────────────┐ │  │
│  │  │ Provider │───▶│ Response │───▶│ Tool Call Parser │ │  │
│  │  │  Client  │    │          │    │                  │ │  │
│  │  └──────────┘    └──────────┘    └────────┬─────────┘ │  │
│  │                                           │           │  │
│  │       ┌───────────────────────────────────▼─────────┐ │  │
│  │       │              Tool Executor                   │ │  │
│  │       │   (User-provided async closure/trait)        │ │  │
│  │       └───────────────────────────────────┬─────────┘ │  │
│  │                                           │           │  │
│  │       ┌───────────────────────────────────▼─────────┐ │  │
│  │       │         Tool Result Formatter               │ │  │
│  │       │   (Back to provider-specific format)        │ │  │
│  │       └─────────────────────────────────────────────┘ │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Components

#### 1. `Tool` struct (new)
```rust
/// A tool/function that can be called by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name (must match function name in executor).
    pub name: String,
    /// Human-readable description for the LLM.
    pub description: String,
    /// JSON Schema for the parameters.
    pub parameters: serde_json::Value,
}
```

#### 2. `ToolCall` struct (new)
```rust
/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this call (for matching results).
    pub id: String,
    /// Name of the tool to call.
    pub name: String,
    /// Arguments as JSON object.
    pub arguments: serde_json::Value,
}
```

#### 3. `ToolResult` struct (new)
```rust
/// Result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// ID of the tool call this result is for.
    pub tool_call_id: String,
    /// Result content (usually JSON string).
    pub content: String,
    /// Whether the tool execution failed.
    pub is_error: bool,
}
```

#### 4. `ToolExecutor` trait (new)
```rust
/// Trait for executing tool calls.
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool call and return the result.
    async fn execute(&self, call: &ToolCall) -> ToolResult;
}

// Convenience impl for closures
impl<F, Fut> ToolExecutor for F
where
    F: Fn(&ToolCall) -> Fut + Send + Sync,
    Fut: Future<Output = ToolResult> + Send,
{
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        (self)(call).await
    }
}
```

#### 5. Extended `ChatRequest`
```rust
/// Chat completion request with optional tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequestWithTools {
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Index to search for context.
    pub index_uid: String,
    /// Whether to stream the response.
    #[serde(default)]
    pub stream: bool,
    /// Available tools (optional).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    /// Tool choice strategy.
    #[serde(default)]
    pub tool_choice: ToolChoice,
    /// Maximum tool call iterations (default: 10).
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoice {
    #[default]
    Auto,
    None,
    Required,
    Specific(String),
}
```

#### 6. Extended `ChatResponse`
```rust
/// Chat completion response with tool metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponseWithTools {
    /// Response content.
    pub content: String,
    /// Sources used (document IDs).
    pub sources: Vec<String>,
    /// Token usage.
    pub usage: Option<Usage>,
    /// Tool calls made during this completion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Whether response was truncated due to max_iterations.
    pub truncated: bool,
}
```

### Provider Conversion

#### OpenAI Format
```json
{
  "tools": [{
    "type": "function",
    "function": {
      "name": "search_products",
      "description": "Search for products",
      "parameters": { "type": "object", "properties": {...} }
    }
  }]
}
```

#### Anthropic Format
```json
{
  "tools": [{
    "name": "search_products",
    "description": "Search for products",
    "input_schema": { "type": "object", "properties": {...} }
  }]
}
```

### Error Handling

| Error | Cause | Recovery |
|-------|-------|----------|
| `ToolsNotSupported` | Provider doesn't support tools | Fail fast |
| `InvalidToolSchema` | Malformed JSON schema | Fail fast with details |
| `ToolCallParseFailed` | Invalid JSON in arguments | Return error as tool result |
| `ToolExecutionFailed` | Executor threw error | Return error as tool result |
| `MaxIterationsExceeded` | Loop limit reached | Return partial response |

### Testing Strategy

1. **Unit tests**: Tool/ToolCall/ToolResult serialization
2. **Integration tests**: With mock LLM server returning tool calls
3. **Provider-specific tests**: OpenAI and Anthropic format conversion
4. **Streaming tests**: Tool call accumulation across chunks

---

## Phase 3: Tasks

### Foundation (Types & Traits)

- [ ] **1.1** Create `src/chat/tools.rs` with `Tool`, `ToolCall`, `ToolResult`, `ToolChoice` structs
  - All structs derive Serialize/Deserialize
  - Add `impl Tool { pub fn new(...) }` builder
  - _Requirements: AC-1, AC-2_

- [ ] **1.2** Add `ToolExecutor` trait and closure impl
  - Use `async_trait` for async execution
  - Add `BoxedToolExecutor` type alias
  - _Requirements: AC-8_

- [ ] **1.3** Add tool-related error variants to `Error` enum
  - `ToolsNotSupported`, `InvalidToolSchema`, `ToolCallParseFailed`, `MaxIterationsExceeded`
  - _Requirements: AC-3, AC-6_

- [ ] **1.4** Unit tests for tool types serialization
  - Test round-trip JSON for all structs
  - Test ToolChoice variants
  - _Requirements: AC-1, AC-2_

### Provider Conversion

- [ ] **2.1** Add `fn convert_tools_to_openai(tools: &[Tool]) -> Vec<...>`
  - Output `async_openai::types::ChatCompletionTool`
  - _Requirements: AC-15_

- [ ] **2.2** Add `fn convert_tools_to_anthropic(tools: &[Tool]) -> Vec<AnthropicTool>`
  - Reuse `AnthropicTool` from HTTP routes
  - _Requirements: AC-16_

- [ ] **2.3** Add `fn parse_openai_tool_calls(response) -> Vec<ToolCall>`
  - Extract from `choices[0].message.tool_calls`
  - _Requirements: AC-7_

- [ ] **2.4** Add `fn parse_anthropic_tool_calls(response) -> Vec<ToolCall>`
  - Extract from `content` blocks with `type: tool_use`
  - _Requirements: AC-7_

- [ ] **2.5** Add `fn format_tool_results_openai(results: &[ToolResult]) -> Vec<Message>`
  - Create tool role messages with tool_call_id
  - _Requirements: AC-9_

- [ ] **2.6** Add `fn format_tool_results_anthropic(results: &[ToolResult]) -> Vec<ContentBlock>`
  - Create tool_result content blocks
  - _Requirements: AC-9_

### Execution Loop

- [ ] **3.1** Add `async fn chat_completion_with_tools<E: ToolExecutor>(...)`
  - Signature: `(&self, request: ChatRequestWithTools, executor: E) -> Result<ChatResponseWithTools, Error>`
  - Implement basic loop without streaming first
  - _Requirements: AC-4, AC-5, AC-7, AC-8, AC-9, AC-10, AC-11_

- [ ] **3.2** Implement max_iterations guard
  - Track iteration count, return `truncated: true` when exceeded
  - _Requirements: AC-11, EC-5_

- [ ] **3.3** Handle parallel tool calls
  - Execute all calls from single response concurrently using `futures::future::join_all`
  - _Requirements: EC-1_

- [ ] **3.4** Integration test with mock server
  - Test full loop: request → tool call → execution → result → final response
  - _Requirements: AC-7, AC-8, AC-9, AC-11_

### Streaming Support

- [ ] **4.1** Add `ToolCallChunk` struct for streaming tool calls
  - Fields: `index`, `id_delta`, `name_delta`, `arguments_delta`
  - _Requirements: AC-12_

- [ ] **4.2** Extend `ChatChunk` with tool call fields
  - Add `tool_calls: Option<Vec<ToolCallChunk>>`
  - _Requirements: AC-12_

- [ ] **4.3** Implement streaming tool call accumulator
  - Buffer partial JSON across `content_block_delta` events
  - Parse complete tool call when `content_block_stop` received
  - _Requirements: AC-13_

- [ ] **4.4** Add `async fn chat_completion_with_tools_stream<E>(...)`
  - Return `Stream<Item = Result<ChatChunk, Error>>`
  - Emit chunks during both generation and tool execution
  - _Requirements: AC-12, AC-14_

### Documentation & Examples

- [ ] **5.1** Add doc comments to all public types
  - Include usage examples in doc comments
  - _Requirements: All_

- [ ] **5.2** Add example in `tests/` showing tool usage pattern
  - Define search tool, implement executor, run completion
  - _Requirements: US-1, US-2_

- [ ] **5.3** Update README.md with tool calling section
  - Show minimal example code
  - Document supported providers
  - _Requirements: US-3_

---

## Appendix: Research Notes

### Crate Analysis

| Crate | GitHub Stars | Last Update | Tool Support | Notes |
|-------|-------------|-------------|--------------|-------|
| `rig-rs` | 4.2k | Active | Excellent | `#[tool]` macro, agent loop |
| `async-openai` | 1.5k | Active | Good | OpenAI types only |
| `anthropic-sdk` | 200 | Active | Good | Claude-specific |
| `rllm` | 50 | Active | Moderate | Unified but young |
| `llm` (graniet) | 100 | Active | Good | Multi-backend |

### Decision Rationale

Chose custom implementation because:
1. **Existing code**: HTTP routes already have 90% of the logic
2. **Dependencies**: Adding `rig-rs` would bring proc-macro crates and potentially conflict with Meilisearch's carefully curated deps
3. **Control**: Meilisearch needs precise control over streaming behavior for the RAG pipeline
4. **Maintenance**: Fewer external deps = easier to maintain

Inspiration from `rig-rs`:
- Clean `Tool` struct with schema
- `ToolExecutor` trait pattern
- Automatic loop management
