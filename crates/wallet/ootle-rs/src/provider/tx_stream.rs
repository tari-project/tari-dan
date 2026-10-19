//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Weak, time::Duration};

use futures::{Stream, StreamExt};
use tari_indexer_client::{
    error::IndexerRestClientError,
    rest_api_client::IndexerRestApiClient,
    sse,
    sse::SseStreamError,
};
use tokio::{sync::watch, time};
use tracing::{debug, error, trace};

#[derive(Debug, Clone)]
pub(crate) struct Paused {
    watch: watch::Sender<bool>,
}

impl Paused {
    /// Sets the paused state.
    /// Returns `true` if the state was changed, `false` if it was already set to the given value.
    pub(crate) fn set_paused(&self, paused: bool) -> bool {
        self.watch.send_if_modified(|v| {
            let prev_paused = *v;
            *v = paused;
            prev_paused != paused
        })
    }

    pub(crate) fn waiter(&self) -> PauseWaiter {
        PauseWaiter {
            rx: self.watch.subscribe(),
        }
    }
}

impl Default for Paused {
    fn default() -> Self {
        let (tx, _rx) = watch::channel(true);
        Self { watch: tx }
    }
}

pub(crate) struct PauseWaiter {
    rx: watch::Receiver<bool>,
}

impl PauseWaiter {
    /// Whether the stream is paused. Marks the state seen, so that only a later change wakes a wait.
    fn is_paused(&mut self) -> bool {
        *self.rx.borrow_and_update()
    }

    /// Waits until the paused state is changed to `true`.
    ///
    /// Returns `true` if the method actually waited for the paused state to become paused,
    /// or `false` if it was already paused when called.
    pub(crate) async fn wait_paused(&mut self) -> bool {
        if self.is_paused() {
            return false;
        }

        loop {
            if self.rx.changed().await.is_err() {
                return true;
            }
            if self.is_paused() {
                break;
            }
        }
        true
    }

    /// Waits until the paused state is changed to `false`.
    ///
    /// Returns `true` if the method actually waited for the paused state to become unpaused,
    /// or `false` if it was already unpaused when called.
    pub(crate) async fn wait_unpaused(&mut self) -> bool {
        if !self.is_paused() {
            return false;
        }

        loop {
            if self.rx.changed().await.is_err() {
                return true;
            }
            if !self.is_paused() {
                break;
            }
        }
        true
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventStreamError {
    #[error("Indexer REST client has been dropped")]
    ClientDropped,
    #[error("Indexer REST client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("SSE stream error: {0}")]
    StreamError(#[from] SseStreamError),
}

pub struct EventStream {
    client: Weak<IndexerRestApiClient>,
    span: tracing::Span,
    paused: PauseWaiter,
}

impl EventStream {
    pub fn new(client: Weak<IndexerRestApiClient>, paused: PauseWaiter) -> Self {
        let span = tracing::debug_span!("EventStream");
        Self { client, span, paused }
    }

    pub fn into_stream(mut self) -> impl Stream<Item = Result<sse::Event, EventStreamError>> {
        async_stream::stream! {
            let client = match self.client.upgrade() {
                Some(client) => client,
                None => {
                    error!("Indexer REST client has been dropped");
                    yield Err(EventStreamError::ClientDropped);
                    return;
                },
            };
            loop {
                let _enter = self.span.enter();
                if self.paused.wait_unpaused().await {
                    debug!("event stream unpaused");
                }

                // Subscribed per connection attempt so that a switch made while paused or retrying, which this
                // connection already reads, is not reported again once it is open.
                let mut endpoint_changes = client.subscribe_endpoint_changes();
                let connected = tokio::select! {
                    result = client.sse_events() => result.map_err(EventStreamError::IndexerClientError),
                    new_endpoint = endpoint_changes.changed() => {
                        debug!(%new_endpoint, "indexer endpoint changed while connecting, reconnecting event stream");
                        continue;
                    },
                };
                let mut events = match connected {
                    Ok(stream) => stream,
                    Err(err) => {
                        error!(%err, "failed to start event stream. Sleeping before retrying");
                        yield Err(err);
                        tokio::select! {
                            _ = time::sleep(Duration::from_secs(5)) => {},
                            new_endpoint = endpoint_changes.changed() => {
                                debug!(%new_endpoint, "indexer endpoint changed, retrying event stream now");
                            },
                        }
                        continue;
                    },
                };

                loop {
                    tokio::select! {
                        _ = self.paused.wait_paused() => {
                            debug!("event stream paused");
                            break;
                        },
                        new_endpoint = endpoint_changes.changed() => {
                            debug!(%new_endpoint, "indexer endpoint changed, reconnecting event stream");
                            break;
                        },
                        event = events.next() =>  {
                            match event {
                                Some(Ok(evt)) => {
                                    trace!(?evt, "received event");
                                    yield Ok(evt);
                                },
                                Some(Err(err)) => {
                                    error!(%err, "error receiving event");
                                    yield Err(EventStreamError::StreamError(err));
                                    break;
                                },
                                None => {
                                    debug!("event stream closed by the indexer, reconnecting");
                                    time::sleep(Duration::from_secs(1)).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::{StreamExt, pin_mut};
    use tari_indexer_client::rest_api_client::IndexerRestApiClient;

    use super::*;
    use crate::provider::test_sse_server::spawn_sse_server;

    async fn next_event_type(
        stream: &mut (impl Stream<Item = Result<sse::Event, EventStreamError>> + Unpin),
    ) -> String {
        let event = time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("no event arrived")
            .expect("stream ended")
            .expect("stream errored");
        event.event_type
    }

    #[tokio::test]
    async fn an_unpause_before_the_first_wait_does_not_count_as_a_pause() {
        let paused = Paused::default();
        let mut waiter = paused.waiter();
        paused.set_paused(false);

        assert!(!waiter.wait_unpaused().await);
        assert!(
            time::timeout(Duration::from_millis(100), waiter.wait_paused())
                .await
                .is_err(),
            "the waiter saw a pause while unpaused"
        );
        paused.set_paused(true);
        time::timeout(Duration::from_millis(100), waiter.wait_paused())
            .await
            .expect("the waiter missed the pause");
    }

    #[tokio::test]
    async fn the_stream_follows_the_client_to_a_new_endpoint() {
        let (url_a, _) = spawn_sse_server(Some("FromA")).await;
        let (url_b, _) = spawn_sse_server(Some("FromB")).await;
        let client = Arc::new(IndexerRestApiClient::connect(url_a).unwrap());
        let paused = Paused::default();
        paused.set_paused(false);

        let stream = EventStream::new(Arc::downgrade(&client), paused.waiter()).into_stream();
        pin_mut!(stream);
        assert_eq!(next_event_type(&mut stream).await, "FromA");

        client.set_endpoint(url_b).unwrap();
        assert_eq!(next_event_type(&mut stream).await, "FromB");
    }

    #[tokio::test]
    async fn a_switch_during_the_retry_wait_connects_to_the_new_endpoint_once() {
        let unreachable = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}/", listener.local_addr().unwrap())
        };
        let (url_b, mut connected_b) = spawn_sse_server(Some("FromB")).await;
        let client = Arc::new(IndexerRestApiClient::connect(unreachable).unwrap());
        let paused = Paused::default();
        paused.set_paused(false);

        let stream = EventStream::new(Arc::downgrade(&client), paused.waiter()).into_stream();
        pin_mut!(stream);
        let connect_err = time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("connect did not fail")
            .expect("stream ended");
        assert!(connect_err.is_err());

        client.set_endpoint(url_b).unwrap();
        // Well inside the 5s retry wait, so only an interrupted wait gets here in time.
        let event = time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("the retry wait was not interrupted")
            .expect("stream ended")
            .expect("stream errored");
        assert_eq!(event.event_type, "FromB");

        let waiting = stream.next();
        pin_mut!(waiting);
        tokio::select! {
            _ = &mut waiting => panic!("stream yielded unexpectedly"),
            _ = time::sleep(Duration::from_millis(300)) => {},
        }
        connected_b.recv().await.unwrap();
        assert!(
            connected_b.try_recv().is_err(),
            "connected to the new endpoint more than once"
        );
    }
}
