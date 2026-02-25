# apex-llm

Model-agnostic LLM client — routing, configuration, request building, response parsing, and output validation.

## Provider compatibility

| Provider | API | Auth | Notes |
|---------|-----|------|-------|
| `LlamaCpp` | OpenAI-compat chat completions | None (local) | llama-server on port 8080; GGUF Q4_K_M |
| `OpenAi` | OpenAI native | `OPENAI_API_KEY` | `/v1/chat/completions` |
| `AzureOpenAi` | Azure OpenAI | `AZURE_API_KEY` | Deployment URL required |

All providers use the same `messages: [{role, content}]` format.  The `OpenAiCompatibleClient` works with llama-server and OpenAI without code changes.

## Modules

- **`lib.rs`**: `LlmProvider`, `ModelConfig`, `RoutingConfig`, `LlmConfig`, `ChatMessage`, `SpendTracker`, `OpenAiCompatibleClient`, and the `LlmClient` async trait.
- **`validators.rs`**: Pure output-validation functions (no async):
  - `extract_json(raw)` — strips markdown fences, extracts raw JSON.
  - `parse_json_response(raw)` — parse JSON with error message.
  - `check_required_fields(value, required)` — returns list of missing fields.
  - `check_field_types(value, types)` — type-check fields against expected types.
  - Content quality checks: `check_non_empty_strings`, `check_numeric_range`, `check_array_length`.

## Routing

`route_task(task, routing, local_available)` determines provider per task:
- `local_only_tasks` — never sent to cloud (data privacy).
- `api_fallback_tasks` — prefer local, fall back to API when unavailable.
- All matching is **case-insensitive** (B191).

## Safe logging

Never log API keys in plaintext.  Use `ModelConfig::redacted_api_key()` (B199) which shows only first 4 + last 4 characters.

## Testability

All pure logic (routing, config validation, response parsing) is testable without external HTTP.  Mock the `LlmClient` trait for integration tests.

## Configuration

```toml
# Set environment variables; never embed in config files:
OPENAI_API_KEY=sk-...
AZURE_API_KEY=...
LLAMACPP_BASE_URL=http://localhost:8080  # default
```
