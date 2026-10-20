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

use crate::{error::IndexerRestClientError, rest_api_client::require_success_status};

pub static MIME_EVENT_STREAM: HeaderValue = HeaderValue::from_static("text/event-stream");

pub struct SseEventStreamBuilder {
    response: reqwest::Response,
}

impl SseEventStreamBuilder {
    pub fn new(response: reqwest::Response) -> Self {
        Self { response }
    }

    pub fn into_stream(self) -> Result<SseEventStream, IndexerRestClientError> {
        let response = require_success_status(self.response)?;
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
    finished: bool,
}

impl SseEventStream {
    pub fn new(bytes_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self {
            bytes_stream: Box::pin(bytes_stream),
            buf: BytesMut::with_capacity(512 * 1024), // 512 KiB
            event_buffer: EventBuffer::new(),
            finished: false,
        }
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
            match this.bytes_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.buf.put(bytes);

                    while let Some(part) = memchr::memchr2(b'\n', b'\r', this.buf.as_ref()) {
                        let mut rest = this.buf.split_off(part + 1);
                        // Handle \r\n as a single line ending: if we split on \r and the
                        // next byte is \n, consume it so it isn't treated as a second break.
                        if this.buf.as_ref().ends_with(b"\r") && rest.first() == Some(&b'\n') {
                            rest.advance(1);
                        }
                        let line = mem::replace(&mut this.buf, rest);

                        match parse_sse_event(&line, &mut this.event_buffer) {
                            Err(e) => {
                                this.finished = true;
                                return Poll::Ready(Some(Err(e)));
                            },
                            Ok(()) => {
                                // Empty line, produce event if possible
                                match this.event_buffer.produce_event() {
                                    Some(event) => {
                                        return Poll::Ready(Some(Ok(event)));
                                    },
                                    None => {
                                        // No event to produce, continue
                                        continue;
                                    },
                                }
                            },
                        }
                    }
                },
                Poll::Ready(Some(Err(e))) => {
                    this.finished = true;
                    break Poll::Ready(Some(Err(e.into())));
                },
                Poll::Ready(None) => break Poll::Ready(None),
                Poll::Pending => break Poll::Pending,
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
