# @tari-project/ootle-wasm-bor

WASM decoders for Tari Ootle values in their CBOR encoding. It carries only the decoders, so it is a fraction of the
size of [`@tari-project/ootle-wasm`](https://www.npmjs.com/package/@tari-project/ootle-wasm), which builds and signs
transactions.

The indexer serves engine values in their CBOR encoding, as base64 strings. These decode one into its JSON, in the
shape of the type of the same name in `@tari-project/ootle-ts-bindings`.

### `decodeSubstateValue(encoded)`

```typescript
import { decodeSubstateValue } from "@tari-project/ootle-wasm-bor";

const value = JSON.parse(decodeSubstateValue(response.state.up.substate));
```

### `decodeExecuteResult(encoded)`

```typescript
import { decodeExecuteResult } from "@tari-project/ootle-wasm-bor";

const result = JSON.parse(decodeExecuteResult(finalized.execution_result));
```

### `decodeTransactionReceipt`, `decodeEvent`, `decodeResource`, `decodeUtxo`

```typescript
import { decodeEvent, decodeTransactionReceipt } from "@tari-project/ootle-wasm-bor";

const receipt = JSON.parse(decodeTransactionReceipt(response.receipt));
const events = response.events.map(([transactionId, event]) => [transactionId, JSON.parse(decodeEvent(event))]);
```

`decodeTransactionReceipt` reads `/transaction-receipts`, `decodeEvent` the `/transactions/events` query and stream,
`decodeResource` `/resources/{address}` and `decodeUtxo` `/utxos`.

All throw if `encoded` is not base64 or its bytes are not a complete encoding of the value.

## License

BSD-3-Clause
