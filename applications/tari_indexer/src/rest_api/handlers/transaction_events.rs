//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::pin::Pin;

use axum::{
    Extension,
    extract::Query,
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response, Sse, sse},
};
use futures::Stream;
use log::*;
use tari_engine_types::{encoded::Encoded, events::Event};
use tari_indexer_client::{
    event::{StreamedTransactionEvent, TransactionEvent},
    types::StreamTransactionEventsRequest,
};
use tari_ootle_transaction::TransactionId;
use tokio_stream::StreamExt;

use crate::{
    event_query_limits::WILDCARD_TOPIC_SCAN_LIMIT,
    network_state_sync::EventFilter,
    rest_api::{context::HandlerContext, handlers::HandlerResult, streaming::disable_proxy_buffering},
    storage_sqlite::SqliteIndexerStore,
    store::{EventQuery, ReadOnlyStore, StoredEvent},
};

const LOG_TARGET: &str = "tari::indexer::rest_api::handlers::transaction_events";

/// Maximum number of events that can be replayed from the database on reconnect.
const MAX_REPLAY_EVENTS: u32 = 10_000;
/// Page size for DB replay queries.
const REPLAY_PAGE_SIZE: u32 = 500;

#[derive(Debug, Clone, Copy)]
struct ReplayLimits {
    page_size: u32,
    /// Pages a reconnect replay may read. A wildcard page can examine `wildcard_scan_limit` rows
    /// and deliver none, so the budget counts pages rather than delivered events.
    max_pages: u32,
    wildcard_scan_limit: u32,
}

const REPLAY_LIMITS: ReplayLimits = ReplayLimits {
    page_size: REPLAY_PAGE_SIZE,
    max_pages: MAX_REPLAY_EVENTS / REPLAY_PAGE_SIZE,
    wildcard_scan_limit: WILDCARD_TOPIC_SCAN_LIMIT,
};

#[utoipa::path(
    get,
    path = "/transactions/events/stream",
    description = "SSE stream of template-emitted transaction events. Supports catch-up via \
                    the `after_id` query parameter or `Last-Event-ID` header.",
    params(
        ("topic" = Option<String>, Query, description = "Filter by event topic"),
        ("substate_id" = Option<String>, Query, description = "Filter by substate ID"),
        ("template_address" = Option<String>, Query, description = "Filter by template address"),
        ("resource_address" = Option<String>, Query, description = "Filter by resource address. \
            Matches only std.resource.* events, which carry the resource as their substate_id. \
            Vault events name no resource: filter those by substate_id (the vault ID)"),
        ("after_id" = Option<i64>, Query, description = "Resume from this event ID (exclusive)"),
    )
)]
pub async fn sse_transaction_events(
    Extension(context): Extension<HandlerContext>,
    headers: HeaderMap,
    Query(req): Query<StreamTransactionEventsRequest>,
) -> HandlerResult<Response> {
    // Resolve after_id: prefer Last-Event-ID header (SSE spec), fall back to query param
    let after_id = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .or(req.after_id);

    let filter = EventFilter {
        topic: req.topic.map(|s| s.into_boxed_str()),
        entity_id: None,
        substate_id: req.substate_id,
        template_address: req.template_address,
        resource_address: req.resource_address,
    };

    if let Some(id) = after_id {
        info!(target: LOG_TARGET, "Client connected to transaction events SSE stream (catch-up from id={})", id);
    } else {
        info!(target: LOG_TARGET, "Client connected to transaction events SSE stream (live)");
    }

    // Subscribe to the broadcast channel BEFORE reading the DB.
    // This ensures no events are missed between the DB read and the live stream.
    let broadcast_rx = context.subscribe_transaction_events();

    type SseStream = Pin<Box<dyn Stream<Item = Result<sse::Event, axum::Error>> + Send>>;

    let event_stream: SseStream = match after_id {
        Some(after_id) => {
            let store = context.read_only_store().clone();
            Box::pin(replay_then_live_stream(store, broadcast_rx, filter, after_id))
        },
        None => Box::pin(live_only_stream(broadcast_rx, filter)),
    };

    let mut response = Sse::new(event_stream).keep_alive(sse::KeepAlive::new()).into_response();
    // `Last-Event-ID` selects where the stream resumes from, so the same URL does not describe the
    // same response.
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("last-event-id"));
    disable_proxy_buffering(response.headers_mut());
    Ok(response)
}

/// Stream that only forwards live broadcast events (no replay).
/// Lagged events are silently skipped (the client has no after_id so there's nothing to replay).
fn live_only_stream(
    broadcast_rx: tokio::sync::broadcast::Receiver<TransactionEvent>,
    filter: EventFilter,
) -> impl Stream<Item = Result<sse::Event, axum::Error>> {
    tokio_stream::wrappers::BroadcastStream::new(broadcast_rx).filter_map(move |res| match res {
        Ok(tx_event) if filter.matches(&tx_event.event) => Some(encode_transaction_event(&tx_event)),
        Ok(_) => None,
        // Lagged: events were dropped from the broadcast buffer, skip and continue
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => {
            warn!(target: LOG_TARGET, "Live-only SSE client lagged, some events were dropped");
            None
        },
    })
}

/// Stream that first replays missed events from the DB, then switches to live.
/// Events are deduplicated during the transition using the event ID.
fn replay_then_live_stream(
    store: ReadOnlyStore<SqliteIndexerStore>,
    broadcast_rx: tokio::sync::broadcast::Receiver<TransactionEvent>,
    filter: EventFilter,
    after_id: i64,
) -> impl Stream<Item = Result<sse::Event, axum::Error>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<sse::Event, axum::Error>>(256);

    tokio::spawn(async move {
        // `run_replay_then_live` only observes a closed stream when it sends, and a filter that
        // matches nothing never sends, so the task also waits on `tx.closed()`.
        tokio::select! {
            result = run_replay_then_live(store, broadcast_rx, filter, after_id, REPLAY_LIMITS, &tx) => {
                if let Err(e) = result {
                    warn!(target: LOG_TARGET, "SSE replay-then-live stream error: {}", e);
                }
            },
            () = tx.closed() => {},
        }
        // tx is dropped here, which closes the stream
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}

async fn run_replay_then_live(
    store: ReadOnlyStore<SqliteIndexerStore>,
    mut broadcast_rx: tokio::sync::broadcast::Receiver<TransactionEvent>,
    filter: EventFilter,
    after_id: i64,
    limits: ReplayLimits,
    tx: &tokio::sync::mpsc::Sender<Result<sse::Event, axum::Error>>,
) -> Result<(), anyhow::Error> {
    // Phase 1: Replay from DB
    let query = replay_query(&filter, limits.wildcard_scan_limit);
    let Some(mut highest_id) = replay_stored(&store, &query, after_id, limits.page_size, limits.max_pages, tx).await?
    else {
        // Client disconnected
        return Ok(());
    };

    debug!(target: LOG_TARGET, "SSE replay complete (highest_id={})", highest_id);

    // Phase 2: Live stream with dedup
    let mut received_live = false;
    loop {
        match broadcast_rx.recv().await {
            Ok(tx_event) => {
                // Skip events we already replayed
                if tx_event.id <= highest_id {
                    continue;
                }
                // Every received event advances the cursor, matched or not, so a lag catch-up
                // starts at the last event received.
                highest_id = tx_event.id;
                received_live = true;
                if !filter.matches(&tx_event.event) {
                    continue;
                }

                let sse_event = encode_transaction_event(&tx_event);
                if tx.send(sse_event).await.is_err() {
                    return Ok(());
                }
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!(target: LOG_TARGET, "SSE broadcast lagged by {} events, catching up from DB (highest_id={})", n, highest_id);
                // Once a live event has arrived, the catch-up spans only the events the broadcast
                // dropped, so it runs to the end: stopping early would lose them without telling
                // the client. Before that, it continues the reconnect replay and keeps its budget.
                let max_pages = if received_live { u32::MAX } else { limits.max_pages };
                match replay_stored(&store, &query, highest_id, limits.page_size, max_pages, tx).await? {
                    Some(caught_up_to) => highest_id = caught_up_to,
                    None => return Ok(()),
                }
            },
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Ok(());
            },
        }
    }
}

/// Sends the stored events matching `query` with id above `after_id`, reading at most
/// `max_pages` pages of `page_size`. Returns the id the client has been brought up to, or `None`
/// if the client disconnected.
async fn replay_stored(
    store: &ReadOnlyStore<SqliteIndexerStore>,
    query: &EventQuery,
    after_id: i64,
    page_size: u32,
    max_pages: u32,
    tx: &tokio::sync::mpsc::Sender<Result<sse::Event, axum::Error>>,
) -> Result<Option<i64>, anyhow::Error> {
    let mut highest_id = after_id;
    for _ in 0..max_pages {
        let page = store.get_events_after_id(query.clone(), highest_id, page_size).await?;

        for stored in &page.events {
            highest_id = stored.id;
            let sse_event = encode_replay_event(stored);
            if tx.send(sse_event).await.is_err() {
                return Ok(None);
            }
        }

        match page.next_cursor {
            Some(cursor) => highest_id = cursor,
            None => break,
        }
    }
    Ok(Some(highest_id))
}

fn replay_query(filter: &EventFilter, wildcard_scan_limit: u32) -> EventQuery {
    EventQuery {
        topic: filter.topic.as_deref().map(str::to_owned),
        substate_id: filter.substate_id.clone(),
        template_address: filter.template_address,
        resource_address: filter.resource_address,
        wildcard_scan_limit,
    }
}

/// Encode a live TransactionEvent (which already carries its DB id) as an SSE event.
fn encode_transaction_event(event: &TransactionEvent) -> Result<sse::Event, axum::Error> {
    to_sse_event(
        event.id,
        event.event.topic(),
        event.transaction_id,
        event.encoded.clone(),
    )
}

/// Encode a replayed event from the DB as an SSE event.
fn encode_replay_event(stored: &StoredEvent) -> Result<sse::Event, axum::Error> {
    to_sse_event(stored.id, &stored.topic, stored.transaction_id, stored.event.clone())
}

/// The SSE event type is the event topic (e.g. "std.vault.withdraw").
fn to_sse_event(
    id: i64,
    topic: &str,
    transaction_id: TransactionId,
    event: Encoded<Event>,
) -> Result<sse::Event, axum::Error> {
    sse::Event::default()
        .event(topic)
        .id(id.to_string())
        .json_data(StreamedTransactionEvent { transaction_id, event })
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tari_template_lib_types::{Metadata, TemplateAddress};
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::storage_sqlite::insert_test_events;

    fn live_event(id: i64, topic: &str) -> TransactionEvent {
        let event = Event::new(None, TemplateAddress::default(), topic.to_string(), Metadata::new());
        TransactionEvent {
            id,
            transaction_id: TransactionId::default(),
            encoded: Encoded::encode(&event).unwrap(),
            event: Arc::new(event),
        }
    }

    /// The JSON `data` of `event` as a client receives it.
    async fn sent_data(event: sse::Event) -> StreamedTransactionEvent {
        let stream = futures::stream::iter([Ok::<_, std::convert::Infallible>(event)]);
        let body = Sse::new(stream).into_response().into_body();
        let text = String::from_utf8(axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec()).unwrap();
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap_or_else(|| panic!("no data line in {text:?}"));
        serde_json::from_str(data).unwrap()
    }

    #[tokio::test]
    async fn live_and_replayed_events_are_sent_as_the_same_streamed_event() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("indexer.db");
        let store = ReadOnlyStore::new(SqliteIndexerStore::try_create(db_path.clone()).unwrap());
        insert_test_events(&db_path, &["std.vault.deposit"]);
        let live = live_event(1, "std.vault.deposit");

        let page = store
            .get_events_after_id(replay_query(&EventFilter::default(), 10), 0, 10)
            .await
            .unwrap();
        let replayed = sent_data(encode_replay_event(&page.events[0]).unwrap()).await;
        let sent_live = sent_data(encode_transaction_event(&live).unwrap()).await;

        assert_eq!(sent_live.transaction_id, live.transaction_id);
        assert_eq!(sent_live.event.decode().unwrap(), *live.event);
        assert_eq!(replayed.transaction_id, live.transaction_id);
        assert_eq!(replayed.event, sent_live.event);
    }

    async fn next_event_id(rx: &mut mpsc::Receiver<Result<sse::Event, axum::Error>>) -> Option<String> {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.ok()??;
        // sse::Event renders as text; the `id:` line carries the event's database id.
        let rendered = format!("{:?}", event.unwrap());
        rendered
            .split("id: ")
            .nth(1)
            .map(|rest| rest.chars().take_while(char::is_ascii_digit).collect())
    }

    #[tokio::test]
    async fn a_lag_catch_up_reaches_dropped_events_far_past_the_last_match() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("indexer.db");
        let store = SqliteIndexerStore::try_create(db_path.clone()).unwrap();
        insert_test_events(&db_path, &["rare.hit"]);

        // A replay budget of one 2-row page: any catch-up from the last match (id 1) that keeps it
        // stops long before the dropped event at id 8.
        let limits = ReplayLimits {
            page_size: 2,
            max_pages: 1,
            wildcard_scan_limit: 2,
        };
        let (publisher, broadcast_rx) = broadcast::channel(4);
        let (tx, mut rx) = mpsc::channel(256);
        let filter = EventFilter {
            topic: Some("rare.*".into()),
            ..Default::default()
        };
        tokio::spawn(async move {
            run_replay_then_live(ReadOnlyStore::new(store), broadcast_rx, filter, 0, limits, &tx).await
        });
        assert_eq!(next_event_id(&mut rx).await.as_deref(), Some("1"));

        // Six unmatched live events, each consumed before the next is sent.
        insert_test_events(&db_path, &["common.miss"; 6]);
        for id in 2..=7 {
            publisher.send(live_event(id, "common.miss")).unwrap();
            while !publisher.is_empty() {
                tokio::task::yield_now().await;
            }
        }

        // Sent without yielding, six events overflow the capacity-4 broadcast: 8 and 9 are dropped.
        let burst = [
            "rare.hit",
            "common.miss",
            "common.miss",
            "common.miss",
            "common.miss",
            "common.miss",
        ];
        insert_test_events(&db_path, &burst);
        for (id, topic) in (8..).zip(burst) {
            publisher.send(live_event(id, topic)).unwrap();
        }
        assert_eq!(next_event_id(&mut rx).await.as_deref(), Some("8"));
    }

    #[tokio::test]
    async fn dropping_a_replay_stream_releases_its_subscription_without_a_matching_event() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteIndexerStore::try_create(dir.path().join("indexer.db")).unwrap();
        let (publisher, broadcast_rx) = broadcast::channel(4);
        let filter = EventFilter {
            topic: Some("never.emitted".into()),
            ..Default::default()
        };

        let stream = replay_then_live_stream(ReadOnlyStore::new(store), broadcast_rx, filter, 0);
        drop(stream);
        publisher.send(live_event(1, "common.miss")).unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            while publisher.receiver_count() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the replay task outlived its response stream");
    }
}
