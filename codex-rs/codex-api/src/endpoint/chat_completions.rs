use super::responses::ResponsesOptions;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::headers::build_session_headers;
use crate::sse::spawn_chat_completions_stream;
use crate::telemetry::SseTelemetry;
use codex_client::EncodedJsonBody;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use crate::auth::SharedAuthProvider;
use http::HeaderValue;
use http::Method;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

/// Streams turns from a Chat Completions endpoint, such as DeepSeek's `/chat/completions`.
pub struct ChatCompletionsClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport> ChatCompletionsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
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
        let body = EncodedJsonBody::encode(&build_chat_completions_body(&request)).map_err(|e| {
            ApiError::Stream(format!("failed to encode chat completions request: {e}"))
        })?;
        let mut headers = extra_headers;
        headers.extend(build_session_headers(session_id, thread_id));
        let stream_response = self
            .session
            .stream_encoded_json_with(
                Method::POST,
                "/chat/completions",
                headers,
                Some(body),
                |req| {
                    req.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                },
            )
            .await?;
        Ok(spawn_chat_completions_stream(
            stream_response,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
        ))
    }
}

/// Converts the Responses-shaped request assembled by the core into a Chat Completions body.
///
/// Developer messages become `system` messages, function calls become assistant `tool_calls`,
/// and function-call outputs become `tool` messages. Reasoning items are replayed as the
/// assistant's `reasoning_content`, which reasoning models such as `deepseek-reasoner` require
/// when a tool call continues a turn. Items and tools without a Chat Completions equivalent are
/// dropped.
pub(crate) fn build_chat_completions_body(request: &ResponsesApiRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending_reasoning: Option<String> = None;
    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let role = match role.as_str() {
                    "developer" => "system",
                    other => other,
                };
                let mut message = json!({"role": role, "content": content_value(content)});
                if role == "assistant"
                    && let Some(reasoning) = pending_reasoning.take()
                {
                    message["reasoning_content"] = Value::String(reasoning);
                }
                messages.push(message);
            }
            ResponseItem::Reasoning {
                summary, content, ..
            } => {
                let text = reasoning_text(summary, content.as_deref());
                if !text.is_empty() {
                    pending_reasoning.get_or_insert_with(String::new).push_str(&text);
                }
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                let tool_call = json!({
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                });
                if !messages.last().is_some_and(is_assistant) {
                    messages.push(json!({"role": "assistant", "content": Value::Null}));
                }
                let message = messages
                    .last_mut()
                    .expect("an assistant message is present");
                match message.get_mut("tool_calls").and_then(Value::as_array_mut) {
                    Some(calls) => calls.push(tool_call),
                    None => message["tool_calls"] = json!([tool_call]),
                }
                if let Some(reasoning) = pending_reasoning.take() {
                    message["reasoning_content"] = Value::String(reasoning);
                }
            }
            ResponseItem::FunctionCallOutput { call_id, output, .. } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id.clone().unwrap_or_default(),
                    "content": output.body.to_text().unwrap_or_default(),
                }));
            }
            _ => {}
        }
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    let tools = function_tools(request);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String(request.tool_choice.clone());
    }
    body
}

fn is_assistant(message: &Value) -> bool {
    message["role"] == "assistant"
}

fn content_value(items: &[ContentItem]) -> Value {
    let text_only = items
        .iter()
        .all(|item| matches!(item, ContentItem::InputText { .. } | ContentItem::OutputText { .. }));
    if text_only {
        return Value::String(
            items
                .iter()
                .filter_map(|item| match item {
                    ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect(),
        );
    }
    Value::Array(items.iter().filter_map(content_part).collect())
}

fn content_part(item: &ContentItem) -> Option<Value> {
    match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
            Some(json!({"type": "text", "text": text}))
        }
        ContentItem::InputImage {
            image: ImageReference::Inline { image_url },
            ..
        } => Some(json!({"type": "image_url", "image_url": {"url": image_url}})),
        _ => None,
    }
}

fn reasoning_text(
    summary: &[ReasoningItemReasoningSummary],
    content: Option<&[ReasoningItemContent]>,
) -> String {
    let mut text = String::new();
    for item in content.unwrap_or_default() {
        match item {
            ReasoningItemContent::ReasoningText { text: piece }
            | ReasoningItemContent::Text { text: piece } => text.push_str(piece),
        }
    }
    if text.is_empty() {
        for ReasoningItemReasoningSummary::SummaryText { text: piece } in summary {
            text.push_str(piece);
        }
    }
    text
}

/// Keeps only function tools, since Chat Completions has no equivalent for hosted tools.
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
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool.get("description").cloned().unwrap_or_default(),
                    "parameters": tool
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ResponsesApiTools;
    use pretty_assertions::assert_eq;
    use serde_json::value::RawValue;

    fn request(input: Vec<ResponseItem>, tools: Option<&str>) -> ResponsesApiRequest {
        ResponsesApiRequest {
            model: "deepseek-chat".to_string(),
            stream: true,
            service_tier: None,
            input,
            tools: tools.map(|tools| {
                ResponsesApiTools::from(RawValue::from_string(tools.to_string()).unwrap().into())
            }),
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
        }
    }

    fn text(role: &str, text: &str) -> ResponseItem {
        ResponseItem::Message {
            encrypted_content: None,
            status: None,
            id: None,
            role: role.to_string(),
            content: vec![if role == "assistant" {
                ContentItem::OutputText {
                    text: text.to_string(),
                    annotations: None,
                    logprobs: None,
                }
            } else {
                ContentItem::InputText {
                    text: text.to_string(),
                }
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    #[test]
    fn converts_roles_tool_calls_outputs_and_function_tools() {
        let input = vec![
            text("developer", "be brief"),
            text("user", "weather?"),
            ResponseItem::Reasoning {
                id: None,
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "need a tool".to_string(),
                }]),
                encrypted_content: None,
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
                call_id: "call_1".to_string(),
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                id: None,
                call_id: Some("call_1".to_string()),
                name: None,
                namespace: None,
                output: codex_protocol::models::FunctionCallOutputPayload::from_text(
                    "sunny".to_string(),
                ),
                internal_chat_message_metadata_passthrough: None,
            },
        ];
        let tools = r#"[
            {"type":"function","name":"get_weather","description":"Look up weather","parameters":{"type":"object"}},
            {"type":"web_search"}
        ]"#;

        let body = build_chat_completions_body(&request(input, Some(tools)));

        assert_eq!(
            body,
            json!({
                "model": "deepseek-chat",
                "messages": [
                    {"role": "system", "content": "be brief"},
                    {"role": "user", "content": "weather?"},
                    {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"},
                        }],
                        "reasoning_content": "need a tool",
                    },
                    {"role": "tool", "tool_call_id": "call_1", "content": "sunny"},
                ],
                "stream": true,
                "stream_options": {"include_usage": true},
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Look up weather",
                        "parameters": {"type": "object"},
                    },
                }],
                "tool_choice": "auto",
            })
        );
    }
}
