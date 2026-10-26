use super::forward::SseDecoder;
use super::forward::SseStep;
use super::forward::forward_sse;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const REQUEST_ID_HEADER: &str = "request-id";

pub(crate) fn spawn_anthropic_messages_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(forward_sse(
        stream_response.bytes,
        tx_event,
        idle_timeout,
        telemetry,
        AnthropicMessagesDecoder::default(),
    ));
    ResponseStream {
        rx_event,
        upstream_request_id,
        interrupt: None,
    }
}

enum ContentBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    Thinking,
}

/// Turns Anthropic Messages stream events into Responses-shaped events.
///
/// Text and tool-input deltas stream out as they arrive. A text or tool-use block is emitted
/// as a finished item when its `content_block_stop` arrives.
#[derive(Default)]
pub(crate) struct AnthropicMessagesDecoder {
    response_id: String,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    stop_reason: Option<String>,
    blocks: BTreeMap<u64, ContentBlock>,
}

impl AnthropicMessagesDecoder {
    fn token_usage(&self) -> TokenUsage {
        let input_tokens = self.input_tokens + self.cache_read_tokens + self.cache_write_tokens;
        TokenUsage {
            input_tokens,
            cached_input_tokens: self.cache_read_tokens,
            cache_write_input_tokens: self.cache_write_tokens,
            output_tokens: self.output_tokens,
            total_tokens: input_tokens + self.output_tokens,
            ..Default::default()
        }
    }

    fn read_usage(&mut self, usage: &Value) {
        if let Some(value) = usage["input_tokens"].as_i64() {
            self.input_tokens = value;
        }
        if let Some(value) = usage["cache_read_input_tokens"].as_i64() {
            self.cache_read_tokens = value;
        }
        if let Some(value) = usage["cache_creation_input_tokens"].as_i64() {
            self.cache_write_tokens = value;
        }
        if let Some(value) = usage["output_tokens"].as_i64() {
            self.output_tokens = value;
        }
    }
}

impl SseDecoder for AnthropicMessagesDecoder {
    fn decode(&mut self, event: &str, data: &str) -> Result<SseStep, ApiError> {
        let value: Value = serde_json::from_str(data).map_err(|error| {
            ApiError::Stream(format!("failed to parse Anthropic stream event: {error}"))
        })?;
        let kind = value["type"].as_str().unwrap_or(event);
        let mut events = Vec::new();
        match kind {
            "message_start" => {
                self.response_id = value["message"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                self.read_usage(&value["message"]["usage"]);
                events.push(ResponseEvent::Created {
                    response_id: Some(self.response_id.clone()),
                });
            }
            "content_block_start" => {
                let index = value["index"].as_u64().unwrap_or(0);
                let block = &value["content_block"];
                let content_block = match block["type"].as_str() {
                    Some("tool_use") => ContentBlock::ToolUse {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        input: String::new(),
                    },
                    Some("thinking" | "redacted_thinking") => ContentBlock::Thinking,
                    _ => ContentBlock::Text(
                        block["text"].as_str().unwrap_or_default().to_string(),
                    ),
                };
                self.blocks.insert(index, content_block);
            }
            "content_block_delta" => {
                let index = value["index"].as_u64().unwrap_or(0);
                let delta = &value["delta"];
                match (delta["type"].as_str(), self.blocks.get_mut(&index)) {
                    (Some("text_delta"), Some(ContentBlock::Text(text))) => {
                        let piece = delta["text"].as_str().unwrap_or_default();
                        text.push_str(piece);
                        events.push(ResponseEvent::OutputTextDelta(piece.to_string()));
                    }
                    (
                        Some("input_json_delta"),
                        Some(ContentBlock::ToolUse { id, input, .. }),
                    ) => {
                        let piece = delta["partial_json"].as_str().unwrap_or_default();
                        input.push_str(piece);
                        events.push(ResponseEvent::ToolCallInputDelta {
                            item_id: id.clone(),
                            call_id: Some(id.clone()),
                            delta: piece.to_string(),
                        });
                    }
                    (Some("thinking_delta"), Some(ContentBlock::Thinking)) => {
                        events.push(ResponseEvent::ReasoningSummaryDelta {
                            delta: delta["thinking"].as_str().unwrap_or_default().to_string(),
                            summary_index: 0,
                        });
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = value["index"].as_u64().unwrap_or(0);
                match self.blocks.remove(&index) {
                    Some(ContentBlock::Text(text)) if !text.is_empty() => {
                        events.push(ResponseEvent::OutputItemDone(ResponseItem::Message {
                            encrypted_content: None,
                            status: None,
                            id: None,
                            role: "assistant".to_string(),
                            content: vec![ContentItem::OutputText {
                                text,
                                annotations: None,
                                logprobs: None,
                            }],
                            phase: None,
                            internal_chat_message_metadata_passthrough: None,
                        }));
                    }
                    Some(ContentBlock::ToolUse { id, name, input }) => {
                        events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                            encrypted_content: None,
                            status: None,
                            id: None,
                            name,
                            namespace: None,
                            arguments: if input.is_empty() {
                                "{}".to_string()
                            } else {
                                input
                            },
                            encrypted_function_args: None,
                            call_id: id,
                            internal_chat_message_metadata_passthrough: None,
                        }));
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                self.read_usage(&value["usage"]);
            }
            "message_stop" => {
                events.push(ResponseEvent::Completed {
                    response_id: self.response_id.clone(),
                    token_usage: Some(self.token_usage()),
                    usage_metadata: None,
                    end_turn: self
                        .stop_reason
                        .as_deref()
                        .map(|reason| reason != "tool_use"),
                });
                return Ok(SseStep::Done(events));
            }
            "error" => {
                let message = value["error"]["message"]
                    .as_str()
                    .unwrap_or("Anthropic stream error");
                return Err(ApiError::Stream(message.to_string()));
            }
            _ => {}
        }
        Ok(SseStep::Continue(events))
    }

    fn finish_without_terminal(&mut self) -> Result<Vec<ResponseEvent>, ApiError> {
        Err(ApiError::Stream(
            "stream closed before message_stop".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use serde_json::json;

    fn decode_all(events: &[(&str, Value)]) -> Vec<ResponseEvent> {
        let mut decoder = AnthropicMessagesDecoder::default();
        let mut out = Vec::new();
        for (name, data) in events {
            match decoder
                .decode(name, &data.to_string())
                .expect("event decodes")
            {
                SseStep::Continue(next) => out.extend(next),
                SseStep::Done(next) => {
                    out.extend(next);
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn streams_text_and_tool_use_then_completes_on_message_stop() {
        let events = decode_all(&[
            (
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1",
                    "usage": {"input_tokens": 12, "cache_read_input_tokens": 3, "output_tokens": 1},
                }}),
            ),
            (
                "content_block_start",
                json!({"type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}}),
            ),
            (
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "Hi"}}),
            ),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 0})),
            (
                "content_block_start",
                json!({"type": "content_block_start", "index": 1,
                    "content_block": {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {}}}),
            ),
            (
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 1,
                    "delta": {"type": "input_json_delta", "partial_json": "{\"city\":"}}),
            ),
            (
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 1,
                    "delta": {"type": "input_json_delta", "partial_json": "\"Paris\"}"}}),
            ),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 1})),
            (
                "message_delta",
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
                    "usage": {"output_tokens": 20}}),
            ),
            ("message_stop", json!({"type": "message_stop"})),
        ]);

        assert_eq!(events.len(), 7);
        assert_matches!(&events[0], ResponseEvent::Created { response_id: Some(id) } if id == "msg_1");
        assert_matches!(&events[1], ResponseEvent::OutputTextDelta(text) if text == "Hi");
        assert_matches!(
            &events[2],
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. })
                if content == &vec![ContentItem::OutputText {
                    text: "Hi".to_string(),
                    annotations: None,
                    logprobs: None,
                }]
        );
        assert_matches!(&events[3], ResponseEvent::ToolCallInputDelta { .. });
        assert_matches!(&events[4], ResponseEvent::ToolCallInputDelta { .. });
        assert_matches!(
            &events[5],
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { name, arguments, call_id, .. })
                if name == "get_weather" && arguments == "{\"city\":\"Paris\"}" && call_id == "toolu_1"
        );
        assert_matches!(
            &events[6],
            ResponseEvent::Completed { response_id, token_usage: Some(usage), end_turn: Some(false), .. }
                if response_id == "msg_1"
                    && usage.input_tokens == 15
                    && usage.cached_input_tokens == 3
                    && usage.output_tokens == 20
        );
    }

    #[test]
    fn error_event_fails_the_stream() {
        let mut decoder = AnthropicMessagesDecoder::default();
        let result = decoder.decode(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_matches!(result, Err(ApiError::Stream(message)) if message == "Overloaded");
    }
}
