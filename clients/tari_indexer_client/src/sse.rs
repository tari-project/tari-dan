//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::Stream;
use reqwest::{header, header::HeaderValue};

use crate::error::IndexerRestClientError;

pub static MIME_EVENT_STREAM: HeaderValue = HeaderValue::from_static("text/event-stream");

/// Longest SSE line accepted, excluding its terminator. `poll_next` produces an event after every `data:` line, so
/// this bounds both the read buffer and the data of a single event.
const MAX_LINE_LEN: usize = 1024 * 1024;

pub struct SseEventStreamBuilder {
    response: reqwest::Response,
}

impl SseEventStreamBuilder {
    pub fn new(response: reqwest::Response) -> Self {
        Self { response }
    }

    pub fn into_stream(self) -> Result<SseEventStream, IndexerRestClientError> {
        let response = match self.response.error_for_status() {
            Ok(resp) => resp,
            Err(err) => {
                return Err(IndexerRestClientError::ErrorResponse {
                    source: err,
                    details: None,
                });
            },
        };
        let content_type = response.headers().get(header::CONTENT_TYPE);
        if content_type != Some(&MIME_EVENT_STREAM) {
            return Err(IndexerRestClientError::InvalidResponse {
                message: format!(
                    "Invalid Content-Type for SSE stream: expected '{:?}', got '{:?}'",
                    MIME_EVENT_STREAM, content_type
                ),
            });
        }

        let stream = response.bytes_stream();
        Ok(SseEventStream::new(stream))
    }
}

impl From<reqwest::Response> for SseEventStreamBuilder {
    fn from(response: reqwest::Response) -> Self {
        Self::new(response)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SseStreamError {
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Event parse error: {details}")]
    EventParseError { details: String },
}

pub struct SseEventStream {
    bytes_stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    event_buffer: EventBuffer,
    buf: BytesMut,
    /// Length of the prefix of `buf` already searched for a line terminator.
    scanned: usize,
    finished: bool,
}

impl SseEventStream {
    pub fn new(bytes_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self {
            bytes_stream: Box::pin(bytes_stream),
            buf: BytesMut::with_capacity(512 * 1024), // 512 KiB
            scanned: 0,
            event_buffer: EventBuffer::new(),
            finished: false,
        }
    }

    /// Splits the next complete line, without its terminator, off the front of `buf`. Fails once a line is
    /// longer than [`MAX_LINE_LEN`], whether or not its terminator has arrived.
    fn next_line(&mut self) -> Result<Option<BytesMut>, SseStreamError> {
        let Some(pos) = memchr::memchr2(b'\n', b'\r', &self.buf[self.scanned..]) else {
            if self.buf.len() > MAX_LINE_LEN {
                return Err(line_too_long());
            }
            self.scanned = self.buf.len();
            return Ok(None);
        };
        let end = self.scanned + pos;
        if end > MAX_LINE_LEN {
            return Err(line_too_long());
        }
        let terminator = self.buf[end];
        let mut line = self.buf.split_to(end + 1);
        line.truncate(end);
        // A `\r\n` pair is one terminator. When the `\n` arrives in a later chunk it reads as an empty line,
        // which produces nothing.
        if terminator == b'\r' && self.buf.first() == Some(&b'\n') {
            self.buf.advance(1);
        }
        self.scanned = 0;
        Ok(Some(line))
    }
}

fn line_too_long() -> SseStreamError {
    SseStreamError::EventParseError {
        details: format!("SSE line exceeds the maximum length of {MAX_LINE_LEN} bytes"),
    }
}

/// Parse line to split field name and value, applying proper trimming.
fn parse_line(line: &str) -> (&str, &str) {
    let (field, value) = line.split_once(':').unwrap_or((line, ""));
    let value = value.strip_prefix(' ').unwrap_or(value);
    (field.trim(), value.trim())
}

fn parse_sse_event(line: &BytesMut, buf_mut: &mut EventBuffer) -> Result<(), SseStreamError> {
    let s = std::str::from_utf8(line).map_err(|e| SseStreamError::EventParseError {
        details: format!("Invalid UTF-8 in SSE event line: {}", e),
    })?;
    let (field, value) = parse_line(s);
    match field {
        "event" => {
            buf_mut.set_event_type(value);
            Ok(())
        },
        "data" => {
            buf_mut.push_data(value);
            Ok(())
        },
        "id" => {
            buf_mut.set_id(value);
            Ok(())
        },
        "retry" => {
            let retry_ms: u64 = value.parse().map_err(|e| SseStreamError::EventParseError {
                details: format!("Invalid retry value in SSE event line: {}", e),
            })?;
            buf_mut.set_retry(Duration::from_millis(retry_ms));
            Ok(())
        },
        // Ignore unknown fields and empty lines
        _ => Ok(()),
    }
}

impl Stream for SseEventStream {
    type Item = Result<Event, SseStreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        let this = self.get_mut();
        loop {
            // Lines already buffered are consumed before reading more, so every event in a chunk is emitted.
            match this.next_line() {
                Ok(Some(line)) => {
                    if let Err(e) = parse_sse_event(&line, &mut this.event_buffer) {
                        this.finished = true;
                        return Poll::Ready(Some(Err(e)));
                    }
                    if let Some(event) = this.event_buffer.produce_event() {
                        return Poll::Ready(Some(Ok(event)));
                    }
                    continue;
                },
                Ok(None) => {},
                Err(e) => {
                    this.finished = true;
                    return Poll::Ready(Some(Err(e)));
                },
            }

            match this.bytes_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => this.buf.put(bytes),
                Poll::Ready(Some(Err(e))) => {
                    this.finished = true;
                    return Poll::Ready(Some(Err(e.into())));
                },
                Poll::Ready(None) => {
                    this.finished = true;
                    return Poll::Ready(None);
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Server-Sent Event representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Event {
    /// A string identifying the type of event described.
    pub event_type: String,
    /// The data field for the message.
    pub data: String,
    /// Last event ID value.
    pub last_event_id: Option<String>,
    /// Reconnection time.
    pub retry: Option<Duration>,
}

impl Event {
    pub fn try_parse_event<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.data)
    }
}

/// Internal buffer used to accumulate lines of an SSE (Server-Sent Events) stream.
///
/// A single [`EventBuffer`] can be used to process the whole stream. [`set_event_type`] and [`push_data`]
/// methods update the state. [`produce_event`] produces a proper [`Event`] and prepares the internal
/// state to process further data.
struct EventBuffer {
    event_type: String,
    data: String,
    last_event_id: Option<String>,
    retry: Option<Duration>,
}

impl EventBuffer {
    /// Creates fresh new [`EventBuffer`].
    #[allow(clippy::new_without_default)]
    fn new() -> Self {
        Self {
            event_type: String::new(),
            data: String::new(),
            last_event_id: None,
            retry: None,
        }
    }

    /// Produces a [`Event`], if current state allow it.
    ///
    /// Reset the internal state to process further data.
    fn produce_event(&mut self) -> Option<Event> {
        if !self.can_produce_event() {
            return None;
        }

        let event_type = mem::take(&mut self.event_type);
        let data = mem::take(&mut self.data);
        let event = Event {
            event_type: Some(event_type)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "message".to_string()),
            data,
            last_event_id: self.last_event_id.clone(),
            retry: self.retry,
        };

        Some(event)
    }

    pub fn can_produce_event(&self) -> bool {
        !self.data.is_empty()
    }

    /// Set the [`Event`]'s type. Overide previous value.
    fn set_event_type(&mut self, event_type: &str) {
        self.event_type.clear();
        self.event_type.push_str(event_type);
    }

    /// Extends internal data with given data.
    fn push_data(&mut self, data: &str) {
        if !self.data.is_empty() {
            self.data.push('\n');
        }
        self.data.push_str(data);
    }

    fn set_id(&mut self, id: &str) {
        self.last_event_id = Some(id.to_string());
    }

    fn set_retry(&mut self, retry: Duration) {
        self.retry = Some(retry);
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};

    use super::*;

    fn stream_of(chunks: Vec<Vec<u8>>) -> SseEventStream {
        SseEventStream::new(stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, reqwest::Error>(Bytes::from(chunk))),
        ))
    }

    async fn collect(mut sse: SseEventStream) -> Vec<Result<Event, SseStreamError>> {
        let mut items = Vec::new();
        while let Some(item) = sse.next().await {
            items.push(item);
        }
        items
    }

    fn data_of(items: &[Result<Event, SseStreamError>]) -> Vec<&str> {
        items
            .iter()
            .map(|item| item.as_ref().expect("event").data.as_str())
            .collect()
    }

    #[tokio::test]
    async fn an_unterminated_line_over_the_limit_fails_the_stream() {
        let chunk = vec![b'x'; 64 * 1024];
        let chunks = vec![chunk; MAX_LINE_LEN / (64 * 1024) + 2];
        let items = collect(stream_of(chunks)).await;
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Err(SseStreamError::EventParseError { .. })));
    }

    #[tokio::test]
    async fn a_data_line_over_the_limit_fails_the_stream() {
        let mut line = b"data:".to_vec();
        line.resize(MAX_LINE_LEN + 1, b'a');
        line.push(b'\n');
        let items = collect(stream_of(vec![line])).await;
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Err(SseStreamError::EventParseError { .. })));
    }

    #[tokio::test]
    async fn a_data_line_at_the_limit_is_an_event() {
        let mut line = b"data:".to_vec();
        line.resize(MAX_LINE_LEN, b'a');
        line.extend_from_slice(b"\r\n");
        let items = collect(stream_of(vec![line])).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap().data.len(), MAX_LINE_LEN - "data:".len());
    }

    #[tokio::test]
    async fn every_event_in_one_chunk_is_emitted() {
        let items = collect(stream_of(vec![b"data: 1\n\ndata: 2\n\ndata: 3\n\n".to_vec()])).await;
        assert_eq!(data_of(&items), ["1", "2", "3"]);
    }

    #[tokio::test]
    async fn a_line_split_into_many_chunks_is_parsed() {
        let mut wire = b"event: big\ndata: ".to_vec();
        wire.resize(wire.len() + 64 * 1024, b'b');
        wire.extend_from_slice(b"\n\n");
        let chunks = wire.chunks(16).map(<[u8]>::to_vec).collect();
        let items = collect(stream_of(chunks)).await;
        assert_eq!(items.len(), 1);
        let event = items[0].as_ref().unwrap();
        assert_eq!(event.event_type, "big");
        assert_eq!(event.data.len(), 64 * 1024);
    }

    #[tokio::test]
    async fn crlf_line_endings_split_across_chunks_are_one_terminator() {
        let chunks = vec![
            b"event: a\r".to_vec(),
            b"\ndata: 1\r".to_vec(),
            b"\n\r\nid: 7\rdata: 2\r\n\r\n".to_vec(),
        ];
        let items = collect(stream_of(chunks)).await;
        assert_eq!(data_of(&items), ["1", "2"]);
        assert_eq!(items[0].as_ref().unwrap().event_type, "a");
        assert_eq!(items[1].as_ref().unwrap().last_event_id.as_deref(), Some("7"));
    }
}
