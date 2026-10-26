# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Lifecycle hooks

Admins can set top-level `allow_managed_hooks_only = true` in
`requirements.toml` to ignore user, project, and session hook configs while
still allowing managed hooks from requirements and managed config layers. This
setting is only supported in `requirements.toml`; putting it in `config.toml`
does not enable managed-hooks-only mode.

## Model providers and wire APIs

A model provider is declared under `[model_providers.<id>]` and selected with
`model_provider`. The `wire_api` field chooses the protocol Codex speaks to the
provider:

- `responses` (default): the OpenAI Responses API at `{base_url}/responses`.
- `chat_completions`: the Chat Completions API at `{base_url}/chat/completions`,
  used by DeepSeek and other OpenAI-compatible providers.
- `anthropic`: the Anthropic Messages API at `{base_url}/messages`, used by Claude
  models. The API key is sent as `x-api-key`.

The older `wire_api = "chat"` value is no longer accepted.

DeepSeek:

```toml
model = "deepseek-chat"
model_provider = "deepseek"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat_completions"
```

Claude:

```toml
model = "claude-sonnet-5-5"
model_provider = "anthropic"

[model_providers.anthropic]
name = "Anthropic"
base_url = "https://api.anthropic.com/v1"
env_key = "ANTHROPIC_API_KEY"
wire_api = "anthropic"
```

Tool calls, streamed text, and token usage work with both new protocols. Reasoning
effort and reasoning summaries are not sent to these providers, so the
`/status` and `exec` headers omit those lines for them.
