//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{Arc, Weak};

use futures::{Stream, StreamExt};
use tari_indexer_client::{
    error::IndexerRestClientError,
    event::{StreamedTransactionEvent, TransactionEvent},
    rest_api_client::IndexerRestApiClient,
    sse::SseStreamError,
    types::StreamTransactionEventsRequest,
};
use tari_ootle_common_types::engine_types::substate::SubstateId;
use tari_template_lib_types::{ResourceAddress, TemplateAddress};
use tracing::error;
use url::Url;

/// Filter for subscribing to transaction events via SSE.
#[derive(Debug, Clone, Default)]
pub struct TransactionEventFilter {
    pub topic: Option<String>,
    pub substate_id: Option<SubstateId>,
    pub template_address: Option<TemplateAddress>,
    /// Filter by resource address. Matches the events that name a resource as their substate —
    /// mints, burns, recalls, freezes, metadata and access-rule changes.
    ///
    /// Vault deposits and withdrawals do not match: a vault event names the vault, not the
    /// resource. To watch a token's transfers, resolve the vaults holding it — a vault's resource
    /// is fixed for its life, so one lookup holds — and set `substate_id` to the vault instead.
    pub resource_address: Option<ResourceAddress>,
    /// Resume the event stream from this event ID (exclusive).
    /// Events with id > after_id will be replayed from the database before switching to live.
    pub after_id: Option<i64>,
}

impl TransactionEventFilter {
    fn into_request(self) -> StreamTransactionEventsRequest {
        StreamTransactionEventsRequest {
            topic: self.topic,
            substate_id: self.substate_id,
            template_address: self.template_address,
            resource_address: self.resource_address,
            after_id: self.after_id,
        }
    }
}

/// Error type for the transaction event stream.
#[derive(Debug, thiserror::Error)]
pub enum EventWatcherError {
    #[error("Indexer REST client has been dropped")]
    ClientDropped,
    #[error("Indexer REST client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("SSE stream error: {0}")]
    StreamError(#[from] SseStreamError),
    #[error("Failed to parse transaction event: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("The indexer served a transaction event that does not decode: {0}")]
    UndecodableEvent(tari_bor::BorError),
    /// The provider was switched to another indexer. Event ids are assigned by each indexer, so the last id
    /// received cannot resume on the new one; open a new stream and choose its starting point afresh.
    #[error(
        "The provider switched to the indexer at {new_endpoint}; event ids from the previous indexer do not apply to \
         it"
    )]
    EndpointChanged { new_endpoint: Url },
}

/// A stream of transaction events from the indexer.
/// Created via `IndexerProvider::watch_events()`.
pub struct TransactionEventStream {
    client: Weak<IndexerRestApiClient>,
    filter: TransactionEventFilter,
}

impl TransactionEventStream {
    pub(crate) fn new(client: Weak<IndexerRestApiClient>, filter: TransactionEventFilter) -> Self {
        Self { client, filter }
    }

    /// Consume into a futures Stream of TransactionEvent items.
    pub fn into_stream(self) -> impl Stream<Item = Result<TransactionEvent, EventWatcherError>> {
        async_stream::stream! {
            let client = match self.client.upgrade() {
                Some(client) => client,
                None => {
                    error!("Indexer REST client has been dropped");
                    yield Err(EventWatcherError::ClientDropped);
                    return;
                },
            };

            let mut endpoint_changes = client.subscribe_endpoint_changes();
            let req = self.filter.into_request();
            let mut events = match client.sse_transaction_events(req).await {
                Ok(stream) => stream,
                Err(err) => {
                    error!(%err, "Failed to start transaction event stream");
                    yield Err(EventWatcherError::IndexerClientError(err));
                    return;
                },
            };

            loop {
                let event = tokio::select! {
                    new_endpoint = endpoint_changes.changed() => {
                        yield Err(EventWatcherError::EndpointChanged { new_endpoint });
                        return;
                    },
                    event = events.next() => event,
                };
                match event {
                    Some(Ok(evt)) => {
                        match evt.try_parse_event::<StreamedTransactionEvent>() {
                            Ok(streamed) => {
                                let event = match streamed.event.decode() {
                                    Ok(event) => event,
                                    Err(err) => {
                                        error!(%err, "Failed to decode transaction event");
                                        yield Err(EventWatcherError::UndecodableEvent(err));
                                        return;
                                    },
                                };
                                // The event ID is transmitted via the SSE `id:` field,
                                // not in the JSON payload.
                                let id = evt.last_event_id.as_deref().and_then(|id| id.parse().ok()).unwrap_or_default();
                                yield Ok(TransactionEvent {
                                    id,
                                    transaction_id: streamed.transaction_id,
                                    event: Arc::new(event),
                                    encoded: streamed.event,
                                });
                            },
                            Err(err) => {
                                error!(%err, "Failed to parse transaction event");
                                yield Err(EventWatcherError::ParseError(err));
                                return;
                            },
                        }
                    },
                    Some(Err(err)) => {
                        error!(%err, "Error receiving transaction event");
                        yield Err(EventWatcherError::StreamError(err));
                        return;
                    },
                    None => {
                        return;
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use futures::{StreamExt, pin_mut};
    use tari_ootle_common_types::engine_types::{encoded::Encoded, events::Event};
    use tari_ootle_transaction::TransactionId;
    use tari_template_lib_types::Metadata;

    use super::*;
    use crate::provider::test_sse_server::{spawn_sse_server, spawn_sse_server_sending};

    #[tokio::test]
    async fn a_streamed_event_is_yielded_decoded_with_its_sse_id() {
        let event = Event::new(
            None,
            TemplateAddress::default(),
            "std.vault.deposit".to_string(),
            Metadata::from_iter([("amount", "100".to_string())]),
        );
        let data = serde_json::to_string(&StreamedTransactionEvent {
            transaction_id: TransactionId::new([1; 32]),
            event: Encoded::encode(&event).unwrap(),
        })
        .unwrap();
        let (url, _) =
            spawn_sse_server_sending(Some(format!("event: {}\nid: 7\ndata: {data}\n\n", event.topic()))).await;
        let client = Arc::new(IndexerRestApiClient::connect(url).unwrap());

        let stream =
            TransactionEventStream::new(Arc::downgrade(&client), TransactionEventFilter::default()).into_stream();
        pin_mut!(stream);
        let streamed = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("no event was streamed")
            .expect("the stream ended")
            .unwrap();

        assert_eq!(streamed.id, 7);
        assert_eq!(streamed.transaction_id, TransactionId::new([1; 32]));
        assert_eq!(*streamed.event, event);
    }

    #[tokio::test]
    async fn switching_endpoint_ends_the_stream() {
        let (url_a, mut connected_a) = spawn_sse_server(None).await;
        let (url_b, _) = spawn_sse_server(None).await;
        let client = Arc::new(IndexerRestApiClient::connect(url_a).unwrap());

        let stream =
            TransactionEventStream::new(Arc::downgrade(&client), TransactionEventFilter::default()).into_stream();
        pin_mut!(stream);
        let first = stream.next();
        pin_mut!(first);
        tokio::select! {
            _ = connected_a.recv() => {},
            _ = &mut first => panic!("stream yielded before the endpoint changed"),
        }

        client.set_endpoint(url_b.as_str()).unwrap();
        let err = tokio::time::timeout(Duration::from_secs(5), first)
            .await
            .expect("stream did not end")
            .expect("stream ended without an error")
            .unwrap_err();
        assert!(
            matches!(&err, EventWatcherError::EndpointChanged { new_endpoint } if new_endpoint.as_str() == url_b),
            "unexpected error: {err}"
        );
        assert!(stream.next().await.is_none());
    }
}
