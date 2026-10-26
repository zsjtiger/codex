use super::responses::ResponsesOptions;
use crate::auth::AuthError;
use crate::auth::AuthHeadersFuture;
use crate::auth::AuthProvider;
use crate::auth::AuthRefreshFuture;
use crate::auth::SharedAuthProvider;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::headers::build_session_headers;
use crate::sse::spawn_anthropic_messages_stream;
use crate::telemetry::SseTelemetry;
use codex_client::EncodedJsonBody;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use http::Method;
use http::header::AUTHORIZATION;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The Messages API requires `max_tokens`. This leaves room for a long answer without
/// exceeding the output limit of the smaller Claude models.
const MAX_OUTPUT_TOKENS: u64 = 8192;

/// Streams turns from the Anthropic Messages API, `POST {base_url}/messages`.
pub struct AnthropicMessagesClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport> AnthropicMessagesClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, Arc::new(AnthropicAuth(auth))),
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            sse_telemetry: sse,
        }
    }

    pub async fn stream_request(
        &self,
        request: ResponsesApiRequest,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ResponsesOptions {
            session_id,
            thread_id,
            extra_headers,
            ..
        } = options;
        let body = EncodedJsonBody::encode(&build_anthropic_body(&request)).map_err(|e| {
            ApiError::Stream(format!("failed to encode Anthropic messages request: {e}"))
        })?;
        let mut headers = extra_headers;
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.extend(build_session_headers(session_id, thread_id));
        let stream_response = self
            .session
            .stream_encoded_json_with(Method::POST, "/messages", headers, Some(body), |req| {
                req.headers.insert(
                    http::header::ACCEPT,
                    HeaderValue::from_static("text/event-stream"),
                );
            })
            .await?;
        Ok(spawn_anthropic_messages_stream(
            stream_response,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
        ))
    }
}

/// Sends the credential the way Anthropic expects it: as `x-api-key`, not as a bearer token.
struct AnthropicAuth(SharedAuthProvider);

impl AnthropicAuth {
    fn move_bearer_to_api_key(headers: &mut HeaderMap) {
        let token = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::to_string);
        if let Some(token) = token
            && let Ok(value) = HeaderValue::from_str(&token)
        {
            headers.remove(AUTHORIZATION);
            headers.insert(HeaderName::from_static("x-api-key"), value);
        }
    }
}

impl AuthProvider for AnthropicAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) -> Result<(), AuthError> {
        self.0.add_auth_headers(headers)?;
        Self::move_bearer_to_api_key(headers);
        Ok(())
    }

    fn resolve_auth_headers(&self) -> AuthHeadersFuture<'_> {
        Box::pin(async move {
            let mut headers = self.0.resolve_auth_headers().await?;
            Self::move_bearer_to_api_key(&mut headers);
            Ok(headers)
        })
    }

    fn refresh_after_unauthorized(&self, rejected_headers: HeaderMap) -> AuthRefreshFuture<'_> {
        self.0.refresh_after_unauthorized(rejected_headers)
    }
}

/// Converts the Responses-shaped request assembled by the core into a Messages API body.
///
/// Developer and system messages move into the top-level `system` field. Function calls become
/// `tool_use` blocks and function-call outputs become `tool_result` blocks inside user turns.
/// Consecutive messages with the same role are merged, since the API requires alternation.
/// Reasoning items are dropped because replaying thinking blocks requires signatures that
/// Codex does not keep.
pub(crate) fn build_anthropic_body(request: &ResponsesApiRequest) -> Value {
    let mut system: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. }
                if role == "system" || role == "developer" =>
            {
                let text = text_of(content);
                if !text.is_empty() {
                    system.push(text);
                }
            }
            ResponseItem::Message { role, content, .. } => {
                let role = if role == "assistant" { "assistant" } else { "user" };
                push_blocks(&mut messages, role, content_blocks(content));
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                let input = serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                push_blocks(
                    &mut messages,
                    "assistant",
                    vec![json!({"type": "tool_use", "id": call_id, "name": name, "input": input})],
                );
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                push_blocks(
                    &mut messages,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": call_id.clone().unwrap_or_default(),
                        "content": output.body.to_text().unwrap_or_default(),
                    })],
                );
            }
            _ => {}
        }
    }

    let mut body = json!({
        "model": request.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "messages": messages,
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = Value::String(system.join("\n\n"));
    }
    let tools = function_tools(request);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!({"type": "auto"});
    }
    body
}

fn push_blocks(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last["role"] == role => {
            if let Some(content) = last["content"].as_array_mut() {
                content.extend(blocks);
            }
        }
        _ => messages.push(json!({"role": role, "content": blocks})),
    }
}

fn text_of(items: &[ContentItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

fn content_blocks(items: &[ContentItem]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                (!text.is_empty()).then(|| json!({"type": "text", "text": text}))
            }
            ContentItem::InputImage {
                image: ImageReference::Inline { image_url },
                ..
            } => Some(image_block(image_url)),
            _ => None,
        })
        .collect()
}

/// Maps a `data:` URL to a base64 image source and anything else to a URL source.
fn image_block(image_url: &str) -> Value {
    if let Some((media_type, data)) = image_url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
    {
        return json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data},
        });
    }
    json!({"type": "image", "source": {"type": "url", "url": image_url}})
}

/// Keeps only function tools, since the Messages API has no equivalent for hosted tools.
fn function_tools(request: &ResponsesApiRequest) -> Vec<Value> {
    let Some(tools) = request.tools.as_ref() else {
        return Vec::new();
    };
    let declared: Vec<Value> = serde_json::from_str(tools.as_raw_value().get()).unwrap_or_default();
    declared
        .into_iter()
        .filter(|tool| tool["type"] == "function")
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool.get("description").cloned().unwrap_or_default(),
                "input_schema": tool
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::value::RawValue;

    #[test]
    fn moves_system_text_and_merges_tool_turns_into_messages_blocks() {
        let request = ResponsesApiRequest {
            model: "claude-sonnet-5-5".to_string(),
            stream: true,
            service_tier: None,
            input: vec![
                ResponseItem::Message {
                    encrypted_content: None,
                    status: None,
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "be brief".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    encrypted_content: None,
                    status: None,
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "weather?".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    encrypted_content: None,
                    status: None,
                    id: None,
                    name: "get_weather".to_string(),
                    namespace: None,
                    arguments: "{\"city\":\"Paris\"}".to_string(),
                    encrypted_function_args: None,
                    call_id: "toolu_1".to_string(),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("toolu_1".to_string()),
                    name: None,
                    namespace: None,
                    output: codex_protocol::models::FunctionCallOutputPayload::from_text(
                        "sunny".to_string(),
                    ),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(ResponsesApiTools::from(
                RawValue::from_string(
                    r#"[{"type":"function","name":"get_weather","description":"Look up weather","parameters":{"type":"object"}}]"#
                        .to_string(),
                )
                .unwrap()
                .into(),
            )),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream_options: None,
            include: Vec::new(),
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
            access_programs: None,
        };

        let body = build_anthropic_body(&request);

        assert_eq!(
            body,
            json!({
                "model": "claude-sonnet-5-5",
                "max_tokens": MAX_OUTPUT_TOKENS,
                "stream": true,
                "system": "be brief",
                "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "weather?"}]},
                    {"role": "assistant", "content": [{
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "get_weather",
                        "input": {"city": "Paris"},
                    }]},
                    {"role": "user", "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": "sunny",
                    }]},
                ],
                "tools": [{
                    "name": "get_weather",
                    "description": "Look up weather",
                    "input_schema": {"type": "object"},
                }],
                "tool_choice": {"type": "auto"},
            })
        );
    }

    #[test]
    fn data_urls_become_base64_image_sources() {
        assert_eq!(
            image_block("data:image/png;base64,AAAA"),
            json!({"type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AAAA",
            }})
        );
        assert_eq!(
            image_block("https://example.com/cat.png"),
            json!({"type": "image", "source": {
                "type": "url", "url": "https://example.com/cat.png",
            }})
        );
    }

    #[test]
    fn credential_is_sent_as_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sk-test"));
        AnthropicAuth::move_bearer_to_api_key(&mut headers);
        assert!(headers.get(AUTHORIZATION).is_none());
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-test");
    }
}
