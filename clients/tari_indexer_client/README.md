# tari_indexer_client

[![Crates.io](https://img.shields.io/crates/v/tari_indexer_client.svg)](https://crates.io/crates/tari_indexer_client)
[![Documentation](https://docs.rs/tari_indexer_client/badge.svg)](https://docs.rs/tari_indexer_client)

Client library for the Tari Ootle indexer node. Provides a typed async REST
client, a GraphQL client, Server-Sent Events (SSE) streaming, and a protobuf
streaming interface for consuming UTXO updates from a running indexer.

## Features

| Feature  | Default | Description                                                              |
|----------|---------|--------------------------------------------------------------------------|
| `client` | yes     | Enables the HTTP clients (`reqwest`-backed REST + SSE + protobuf stream) |
| `ts`     | no      | Generates TypeScript type bindings via `ts-rs`                           |
| `utoipa` | no      | Adds `utoipa` OpenAPI annotations to request/response types              |

## Key types

| Type                   | Description                                                      |
|------------------------|------------------------------------------------------------------|
| `IndexerRestApiClient` | Async REST client — the primary way to interact with the indexer |
| `SseEventStream`       | Server-Sent Events stream for live indexer events                |
| `ProtobufStream`       | Streaming protobuf reader for bulk UTXO update feeds             |

## API Endpoints

### Identity & Status

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `wait_until_ready()` | GET | `/wait-until-ready` | Blocks until the indexer is ready to serve requests |

### Network

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `get_network_info()` | GET | `/network` | Get network info |
| `get_network_sync_state()` | GET | `/network/stats` | Get network sync stats |
| `get_connections()` | GET | `/network/connections` | Get active peer connections |
| `get_epoch_manager_stats()` | GET | `/epoch-manager/stats` | Get epoch manager stats |

### Substates

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `get_substate(id, req)` | GET | `/substates/{substate_id}` | Fetch a substate by ID |
| `fetch_substates(req)` | POST | `/substates/fetch` | Fetch several substates by their IDs |

### Transactions

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `submit_transaction(req)` | POST | `/transactions` | Submit a transaction to validators |
| `submit_transaction_dry_run(req)` | POST | `/transactions/dry-run` | Submit a transaction as a dry-run (rate limited) |
| `get_transaction_result(req)` | GET | `/transactions/{transaction_id}/result` | Get the result of a submitted transaction |
| `list_recent_transactions(req)` | GET | `/transactions/recent` | List recent transactions |
| `query_transaction_events(req)` | GET | `/transactions/events` | Query and filter transaction events by substate ID and/or topic |
| `sse_transaction_events(req)` | GET (SSE) | `/transactions/events/stream` | Subscribe to a live stream of template-emitted transaction events |

### Templates

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `get_template_definition(address)` | GET | `/templates/{template_address}` | Fetch a template definition by its address |

### Resources

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `get_resource(address)` | GET | `/resources/{resource_address}` | Fetch a resource by ID |

### UTXOs

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `list_utxos(req)` | GET | `/utxos` | List full UTXO data |
| `get_utxos(req)` | POST | `/utxos/fetch` | Get full UTXO data for a list of UTXO IDs |
| `stream_utxo_updates_protobuf(req)` | POST | `/utxos/stream` | Stream UTXO updates via protobuf encoding |

### Transaction Receipts

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `list_transaction_receipts(req)` | GET | `/transaction-receipts` | List transaction receipts |
| `get_transaction_receipt(address)` | GET | `/transaction-receipts/{address}` | Get a transaction receipt by address |

### Events

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `sse_events()` | GET (SSE) | `/events` | Subscribe to live indexer events |

## Example

```rust
use tari_indexer_client::{connect_rest, types::{GetSubstateRequest, ListSubstatesRequest}};
use tari_engine_types::substate::SubstateId;
use futures::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = connect_rest("http://localhost:18300")?;

    // Fetch a specific substate by ID
    let response = client.get_substate(GetSubstateRequest {
        substate_id: some_substate_id,
        version: None,
    }).await?;

    println!("{:?}", response.substate);

    // Stream live events
    let mut events = client.sse_events().await?;

    while let Some(event) = events.next().await {
        println!("Received event: {:?}", event?);
    }

    Ok(())
}
```

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
