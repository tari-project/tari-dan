# @tari-project/ootle-wasm-bor

WASM decoders for Tari Ootle values in their CBOR encoding. It carries only the decoders, so it is a fraction of the
size of [`@tari-project/ootle-wasm`](https://www.npmjs.com/package/@tari-project/ootle-wasm), which builds and signs
transactions.

The indexer serves substate values and execution results in their CBOR encoding, as base64 strings. These decode
one into its JSON, in the shape of the `SubstateValue` and `ExecuteResult` types in `@tari-project/ootle-ts-bindings`.

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

Both throw if `encoded` is not base64 or its bytes are not a complete encoding of the value.

## License

BSD-3-Clause
