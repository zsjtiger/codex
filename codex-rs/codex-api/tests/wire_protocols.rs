#![allow(clippy::expect_used)]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use codex_api::AnthropicMessagesClient;
use codex_api::ApiError;
use codex_api::AuthError;
use codex_api::AuthProvider;
use codex_api::ChatCompletionsClient;
use codex_api::Provider;
use codex_api::ResponseEvent;
use codex_api::ResponseStream;
use codex_api::ResponsesApiRequest;
use codex_api::ResponsesApiTools;
use codex_api::ResponsesOptions;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::RequestBody;
use codex_client::Response;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;

/// Replays canned SSE bodies in order and records every request it receives.
#[derive(Clone)]
struct ScriptedTransport {
    replies: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl ScriptedTransport {
    fn new(replies: Vec<String>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.requests.lock().expect("requests mutex"))
    }
}

impl HttpTransport for ScriptedTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, req: Request) -> Result<StreamResponse, TransportError> {
        self.requests.lock().expect("requests mutex").push(req);
        let body = {
            let mut replies = self.replies.lock().expect("replies mutex");
            assert!(
                !replies.is_empty(),
                "transport received an unscripted request"
            );
            replies.remove(0)
        };
        let stream = futures::stream::iter(vec![Ok::<Bytes, TransportError>(Bytes::from(body))]);
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream),
        })
    }
}

#[derive(Clone, Default)]
struct NoAuth;

impl AuthProvider for NoAuth {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) -> Result<(), AuthError> {
        Ok(())
    }
}

fn provider(name: &str, base_url: &str) -> Provider {
    Provider {
        name: name.to_string(),
        base_url: base_url.to_string(),
        query_params: None,
        headers: HeaderMap::new(),
        retry: codex_api::RetryConfig {
            max_attempts: 1,
            base_delay: Duration::from_millis(1),
            retry_429: false,
            retry_5xx: false,
            retry_transport: true,
        },
        stream_idle_timeout: Duration::from_millis(500),
    }
}

fn get_goal_tool() -> ResponsesApiTools {
    let declared = json!([{
        "type": "function",
        "name": "get_goal",
        "description": "Read the current goal",
        "parameters": {"type": "object", "properties": {}},
    }]);
    ResponsesApiTools::from(Arc::<RawValue>::from(
        RawValue::from_string(declared.to_string()).expect("valid tool JSON"),
    ))
}

fn request(model: &str, input: Vec<ResponseItem>) -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: model.to_string(),
        stream: true,
        service_tier: None,
        input,
        tools: Some(get_goal_tool()),
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

fn user_text(text: &str) -> ResponseItem {
    ResponseItem::Message {
        encrypted_content: None,
        status: None,
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn function_call(call_id: &str, name: &str, arguments: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        encrypted_content: None,
        status: None,
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: arguments.to_string(),
        encrypted_function_args: None,
        call_id: call_id.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn function_output(call_id: &str, output: &str) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        id: None,
        call_id: Some(call_id.to_string()),
        name: None,
        namespace: None,
        output: FunctionCallOutputPayload::from_text(output.to_string()),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn chat_sse(chunks: Vec<Value>) -> String {
    let mut body: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    body
}

fn anthropic_sse(events: Vec<(&str, Value)>) -> String {
    events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect()
}

async fn collect(mut stream: ResponseStream) -> Vec<ResponseEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.rx_event.recv().await {
        let event: Result<ResponseEvent, ApiError> = event;
        events.push(event.expect("stream event should succeed"));
    }
    events
}

fn body_json(request: &Request) -> Value {
    let Some(RequestBody::EncodedJson(body)) = request.body.as_ref() else {
        panic!("expected a prepared request body");
    };
    serde_json::from_slice(body.as_bytes()).expect("request body is JSON")
}

fn function_call_from(events: &[ResponseEvent]) -> (String, String, String) {
    events
        .iter()
        .find_map(|event| match event {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            }) => Some((call_id.clone(), name.clone(), arguments.clone())),
            _ => None,
        })
        .expect("a finished function call is emitted")
}

fn streamed_text(events: &[ResponseEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            ResponseEvent::OutputTextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn chat_completions_round_trips_a_function_call() -> Result<()> {
    let tool_turn = chat_sse(vec![
        json!({"id": "c1", "choices": [{"index": 0, "delta": {"role": "assistant",
            "tool_calls": [{"index": 0, "id": "call_goal_1", "type": "function",
                "function": {"name": "get_goal", "arguments": ""}}]}}]}),
        json!({"id": "c1", "choices": [{"index": 0, "delta": {
            "tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}}]}),
        json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
    ]);
    let text_turn = chat_sse(vec![json!({"id": "c2", "choices": [
        {"index": 0, "delta": {"content": "Goal checked."}, "finish_reason": "stop"}]})]);
    let transport = ScriptedTransport::new(vec![tool_turn, text_turn]);
    let base_url = "https://api.deepseek.com";
    let client = ChatCompletionsClient::new(
        transport.clone(),
        provider("deepseek", base_url),
        Arc::new(NoAuth),
    );

    let user = user_text("Check the goal.");
    let first = collect(
        client
            .stream_request(
                request("deepseek-chat", vec![user.clone()]),
                ResponsesOptions::default(),
            )
            .await?,
    )
    .await;
    assert_eq!(
        function_call_from(&first),
        (
            "call_goal_1".to_string(),
            "get_goal".to_string(),
            "{}".to_string()
        )
    );
    assert!(first.iter().any(|event| matches!(
        event,
        ResponseEvent::Completed {
            end_turn: Some(false),
            ..
        }
    )));

    let second_input = vec![
        user,
        function_call("call_goal_1", "get_goal", "{}"),
        function_output("call_goal_1", r#"{"goal":null}"#),
    ];
    let second = collect(
        client
            .stream_request(
                request("deepseek-chat", second_input),
                ResponsesOptions::default(),
            )
            .await?,
    )
    .await;
    assert_eq!(streamed_text(&second), "Goal checked.");

    let requests = transport.take_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|r| r.url.ends_with("/chat/completions"))
    );

    let first_body = body_json(&requests[0]);
    assert_eq!(first_body["tools"][0]["function"]["name"], "get_goal");

    let second_body = body_json(&requests[1]);
    let messages = second_body["messages"].as_array().expect("messages array");
    assert_eq!(
        messages[1]["tool_calls"][0],
        json!({"id": "call_goal_1", "type": "function",
            "function": {"name": "get_goal", "arguments": "{}"}})
    );
    assert_eq!(
        messages[2],
        json!({"role": "tool", "tool_call_id": "call_goal_1", "content": r#"{"goal":null}"#})
    );
    Ok(())
}

#[tokio::test]
async fn anthropic_messages_round_trips_a_tool_use() -> Result<()> {
    let tool_turn = anthropic_sse(vec![
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_1", "usage": {"input_tokens": 9, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "tool_use", "id": "toolu_goal_1", "name": "get_goal", "input": {}}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 3}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    let text_turn = anthropic_sse(vec![
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_2", "usage": {"input_tokens": 20, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "Goal checked."}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 4}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    let transport = ScriptedTransport::new(vec![tool_turn, text_turn]);
    let client = AnthropicMessagesClient::new(
        transport.clone(),
        provider("anthropic", "https://api.anthropic.com/v1"),
        Arc::new(NoAuth),
    );

    let user = user_text("Check the goal.");
    let first = collect(
        client
            .stream_request(
                request("claude-sonnet-5-5", vec![user.clone()]),
                ResponsesOptions::default(),
            )
            .await?,
    )
    .await;
    assert_eq!(
        function_call_from(&first),
        (
            "toolu_goal_1".to_string(),
            "get_goal".to_string(),
            "{}".to_string()
        )
    );
    assert!(first.iter().any(|event| matches!(
        event,
        ResponseEvent::Completed {
            end_turn: Some(false),
            ..
        }
    )));

    let second_input = vec![
        user,
        function_call("toolu_goal_1", "get_goal", "{}"),
        function_output("toolu_goal_1", r#"{"goal":null}"#),
    ];
    let second = collect(
        client
            .stream_request(
                request("claude-sonnet-5-5", second_input),
                ResponsesOptions::default(),
            )
            .await?,
    )
    .await;
    assert_eq!(streamed_text(&second), "Goal checked.");

    let requests = transport.take_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.url.ends_with("/messages")));

    let first_body = body_json(&requests[0]);
    assert_eq!(first_body["tools"][0]["name"], "get_goal");

    let second_body = body_json(&requests[1]);
    let messages = second_body["messages"].as_array().expect("messages array");
    assert_eq!(
        messages[1]["content"][0],
        json!({"type": "tool_use", "id": "toolu_goal_1", "name": "get_goal", "input": {}})
    );
    assert_eq!(
        messages[2]["content"][0],
        json!({"type": "tool_result", "tool_use_id": "toolu_goal_1", "content": r#"{"goal":null}"#})
    );
    Ok(())
}
