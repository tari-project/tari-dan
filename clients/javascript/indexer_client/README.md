# Tari Ootle Indexer API Client (JavaScript/TypeScript)

## Overview

TypeScript client library for interacting with the Tari Ootle Indexer REST API.

## Installation

```bash
npm install @ootle/indexer-client
```

## Usage

```typescript
import { IndexerClient } from "@ootle/indexer-client";

const client = IndexerClient.usingFetchTransport("http://localhost:18300");

// Fetch a substate
const substate = await client.substatesGet(substateId, { version: null });

// Submit a transaction
const result = await client.submitTransaction({ transaction, is_dry_run: false });
```

## API Endpoints

### Identity & Status

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `identityGet()` | GET | `/identity` | Get indexer network identity information |
| `waitUntilReady()` | GET | `/wait-until-ready` | Blocks until the indexer is ready to serve requests |

### Network

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `networkInfo()` | GET | `/network` | Get network info |
| `networkStats()` | GET | `/network/stats` | Get network sync stats |
| `getConnections()` | GET | `/network/connections` | Get active peer connections |
| `epochManagerStats()` | GET | `/epoch-manager/stats` | Get epoch manager stats |

### Substates

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `substatesGet(id, params)` | GET | `/substates/{substate_id}` | Fetch a substate by ID |
| `fetchSubstates(params)` | POST | `/substates/fetch` | Fetch several substates by their IDs |

### Transactions

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `submitTransaction(params)` | POST | `/transactions` | Submit a transaction to validators |
| `getTransactionResult(id)` | GET | `/transactions/{transaction_id}/result` | Get the result of a submitted transaction |
| `listRecentTransactions(params)` | GET | `/transactions/recent` | List recent transactions |
| `queryTransactionEvents(params)` | GET | `/transactions/events` | Query and filter transaction events by substate ID and/or topic |
| `streamTransactionEvents(params, options)` | GET (SSE) | `/transactions/events/stream` | Subscribe to a live stream of template-emitted transaction events |

### Templates

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `templatesGet(address)` | GET | `/templates/{template_address}` | Fetch a template definition by its address |

### Resources

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `resourcesGet(address)` | GET | `/resources/{resource_address}` | Fetch a resource by ID |

### Transaction Receipts

| Method | HTTP | Path | Description |
|--------|------|------|-------------|
| `listTransactionReceipts(params)` | GET | `/transaction-receipts` | List transaction receipts |
| `getTransactionReceipt(address)` | GET | `/transaction-receipts/{address}` | Get a transaction receipt by address |

## License

BSD-3-Clause. Copyright 2024 The Tari Project.
