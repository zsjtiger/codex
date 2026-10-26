use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;

/// Outcome of decoding one SSE event.
pub(crate) enum SseStep {
    /// Keep reading the stream after emitting these events.
    Continue(Vec<ResponseEvent>),
    /// The protocol's terminal event: emit these events and stop reading.
    Done(Vec<ResponseEvent>),
}

/// Protocol-specific translation of an SSE stream into Responses-shaped events.
pub(crate) trait SseDecoder: Send {
    fn decode(&mut self, event: &str, data: &str) -> Result<SseStep, ApiError>;

    /// Called when the transport closes before the protocol's terminal event.
    fn finish_without_terminal(&mut self) -> Result<Vec<ResponseEvent>, ApiError>;
}

/// Reads `stream` as server-sent events, decodes them with `decoder`, and forwards the
/// resulting events until the decoder reports a terminal event, the stream fails, or the
/// receiver is dropped.
pub(crate) async fn forward_sse<D: SseDecoder>(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    mut decoder: D,
) {
    let mut stream = stream.eventsource();
    loop {
        let start = Instant::now();
        let response = tokio::select! {
            biased;
            _ = tx_event.closed() => return,
            response = timeout(idle_timeout, stream.next()) => response,
        };
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let step = match response {
            Ok(Some(Ok(sse))) => decoder.decode(&sse.event, &sse.data),
            Ok(Some(Err(e))) => Err(ApiError::Stream(e.to_string())),
            Ok(None) => decoder.finish_without_terminal().map(SseStep::Done),
            Err(_) => Err(ApiError::Stream("idle timeout waiting for SSE".into())),
        };
        let (events, done) = match step {
            Ok(SseStep::Continue(events)) => (events, false),
            Ok(SseStep::Done(events)) => (events, true),
            Err(error) => {
                let _ = tx_event.send(Err(error)).await;
                return;
            }
        };
        for event in events {
            if tx_event.send(Ok(event)).await.is_err() {
                return;
            }
        }
        if done {
            return;
        }
    }
}
