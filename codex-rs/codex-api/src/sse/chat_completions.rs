use super::forward::SseDecoder;
use super::forward::SseStep;
use super::forward::forward_sse;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const REQUEST_ID_HEADER: &str = "x-request-id";

pub(crate) fn spawn_chat_completions_stream(
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
        ChatCompletionsDecoder::default(),
    ));
    ResponseStream {
        rx_event,
        upstream_request_id,
        interrupt: None,
    }
}

#[derive(Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Turns Chat Completions chunks into Responses-shaped events.
///
/// Text and reasoning deltas stream out as they arrive. Message and function-call items are
/// emitted once, on `[DONE]`, so each item carries its complete content.
#[derive(Default)]
pub(crate) struct ChatCompletionsDecoder {
    started: bool,
    response_id: String,
    text: String,
    reasoning: String,
    tool_calls: BTreeMap<u64, PendingToolCall>,
    finish_reason: Option<String>,
    token_usage: Option<TokenUsage>,
}

impl ChatCompletionsDecoder {
    fn complete(&mut self) -> Vec<ResponseEvent> {
        let mut events = Vec::new();
        if !self.reasoning.is_empty() {
            events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                id: None,
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: std::mem::take(&mut self.reasoning),
                }]),
                encrypted_content: None,
                internal_chat_message_metadata_passthrough: None,
            }));
        }
        if !self.text.is_empty() || self.tool_calls.is_empty() {
            events.push(ResponseEvent::OutputItemDone(ResponseItem::Message {
                encrypted_content: None,
                status: None,
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: std::mem::take(&mut self.text),
                    annotations: None,
                    logprobs: None,
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }));
        }
        for (index, call) in std::mem::take(&mut self.tool_calls) {
            let call_id = if call.id.is_empty() {
                format!("call_{index}")
            } else {
                call.id
            };
            events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                encrypted_content: None,
                status: None,
                id: None,
                name: call.name,
                namespace: None,
                arguments: if call.arguments.is_empty() {
                    "{}".to_string()
                } else {
                    call.arguments
                },
                encrypted_function_args: None,
                call_id,
                internal_chat_message_metadata_passthrough: None,
            }));
        }
        events.push(ResponseEvent::Completed {
            response_id: std::mem::take(&mut self.response_id),
            token_usage: self.token_usage.take(),
            usage_metadata: None,
            end_turn: self
                .finish_reason
                .as_deref()
                .map(|reason| reason != "tool_calls"),
        });
        events
    }
}

impl SseDecoder for ChatCompletionsDecoder {
    fn decode(&mut self, _event: &str, data: &str) -> Result<SseStep, ApiError> {
        if data.trim() == "[DONE]" {
            return Ok(SseStep::Done(self.complete()));
        }
        let chunk: Value = serde_json::from_str(data).map_err(|error| {
            ApiError::Stream(format!("failed to parse chat completion chunk: {error}"))
        })?;
        if let Some(error) = chunk.get("error") {
            let message = error["message"].as_str().unwrap_or("provider error");
            return Err(ApiError::Stream(message.to_string()));
        }

        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            self.response_id = chunk["id"].as_str().unwrap_or_default().to_string();
            events.push(ResponseEvent::Created {
                response_id: Some(self.response_id.clone()),
            });
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| !usage.is_null()) {
            self.token_usage = Some(token_usage(usage));
        }

        for choice in chunk["choices"].as_array().into_iter().flatten() {
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.to_string());
            }
            let delta = &choice["delta"];
            if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
                self.text.push_str(text);
                events.push(ResponseEvent::OutputTextDelta(text.to_string()));
            }
            if let Some(text) = delta["reasoning_content"]
                .as_str()
                .filter(|text| !text.is_empty())
            {
                self.reasoning.push_str(text);
                events.push(ResponseEvent::ReasoningSummaryDelta {
                    delta: text.to_string(),
                    summary_index: 0,
                });
            }
            for tool_call in delta["tool_calls"].as_array().into_iter().flatten() {
                let index = tool_call["index"].as_u64().unwrap_or(0);
                let call = self.tool_calls.entry(index).or_default();
                if let Some(id) = tool_call["id"].as_str().filter(|id| !id.is_empty()) {
                    call.id = id.to_string();
                }
                if let Some(name) = tool_call["function"]["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                {
                    call.name = name.to_string();
                }
                if let Some(arguments) = tool_call["function"]["arguments"].as_str() {
                    call.arguments.push_str(arguments);
                    events.push(ResponseEvent::ToolCallInputDelta {
                        item_id: call.id.clone(),
                        call_id: Some(call.id.clone()),
                        delta: arguments.to_string(),
                    });
                }
            }
        }
        Ok(SseStep::Continue(events))
    }

    fn finish_without_terminal(&mut self) -> Result<Vec<ResponseEvent>, ApiError> {
        if self.finish_reason.is_some() {
            Ok(self.complete())
        } else {
            Err(ApiError::Stream(
                "stream closed before chat completion finished".into(),
            ))
        }
    }
}

fn token_usage(usage: &Value) -> TokenUsage {
    let input_tokens = usage["prompt_tokens"].as_i64().unwrap_or(0);
    let output_tokens = usage["completion_tokens"].as_i64().unwrap_or(0);
    TokenUsage {
        input_tokens,
        cached_input_tokens: usage["prompt_cache_hit_tokens"]
            .as_i64()
            .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_i64())
            .unwrap_or(0),
        output_tokens,
        reasoning_output_tokens: usage["completion_tokens_details"]["reasoning_tokens"]
            .as_i64()
            .unwrap_or(0),
        total_tokens: usage["total_tokens"]
            .as_i64()
            .unwrap_or(input_tokens + output_tokens),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    fn decode_all(chunks: &[&str]) -> Vec<ResponseEvent> {
        let mut decoder = ChatCompletionsDecoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            match decoder.decode("message", chunk).expect("chunk decodes") {
                SseStep::Continue(next) => events.extend(next),
                SseStep::Done(next) => {
                    events.extend(next);
                    break;
                }
            }
        }
        events
    }

    #[test]
    fn streams_text_and_tool_calls_then_completes_on_done() {
        let events = decode_all(&[
            r#"{"id":"c1","choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"}}]}"#,
            r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
            r#"{"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_9","type":"function","function":{"name":"get_weather","arguments":"{\"city\""}}]}}]}"#,
            r#"{"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"Paris\"}"}}]}}]}"#,
            r#"{"id":"c1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"id":"c1","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_cache_hit_tokens":4}}"#,
            "[DONE]",
        ]);

        assert_eq!(events.len(), 8);
        assert_matches!(&events[0], ResponseEvent::Created { response_id: Some(id) } if id == "c1");
        assert_matches!(&events[1], ResponseEvent::OutputTextDelta(text) if text == "Hel");
        assert_matches!(&events[2], ResponseEvent::OutputTextDelta(text) if text == "lo");
        assert_matches!(&events[3], ResponseEvent::ToolCallInputDelta { call_id: Some(id), .. } if id == "call_9");
        assert_matches!(&events[4], ResponseEvent::ToolCallInputDelta { .. });
        assert_matches!(
            &events[5],
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. })
                if content == &vec![ContentItem::OutputText {
                    text: "Hello".to_string(),
                    annotations: None,
                    logprobs: None,
                }]
        );
        assert_matches!(
            &events[6],
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { name, arguments, call_id, .. })
                if name == "get_weather" && arguments == "{\"city\":\"Paris\"}" && call_id == "call_9"
        );
        assert_matches!(
            &events[7],
            ResponseEvent::Completed { response_id, token_usage: Some(usage), end_turn: Some(false), .. }
                if response_id == "c1"
                    && usage.input_tokens == 10
                    && usage.cached_input_tokens == 4
                    && usage.output_tokens == 5
                    && usage.total_tokens == 15
        );
    }

    #[test]
    fn reasoning_content_is_surfaced_and_kept_as_a_reasoning_item() {
        let events = decode_all(&[
            r#"{"id":"c2","choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}"#,
            r#"{"id":"c2","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);

        assert_matches!(&events[1], ResponseEvent::ReasoningSummaryDelta { delta, .. } if delta == "think");
        assert_matches!(
            &events[3],
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning { content: Some(content), .. })
                if content == &vec![ReasoningItemContent::ReasoningText { text: "think".to_string() }]
        );
        assert_matches!(&events[5], ResponseEvent::Completed { end_turn: Some(true), .. });
    }

    #[test]
    fn stream_closing_before_finish_is_an_error() {
        let mut decoder = ChatCompletionsDecoder::default();
        decoder
            .decode(
                "message",
                r#"{"id":"c3","choices":[{"index":0,"delta":{"content":"partial"}}]}"#,
            )
            .expect("chunk decodes");
        assert!(decoder.finish_without_terminal().is_err());
    }
}
