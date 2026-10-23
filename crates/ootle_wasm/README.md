# ootle-wasm

Rust → WASM library providing client-side crypto operations for Tari Ootle L2.

## Architecture

- `core/` — Pure Rust library. All crypto/encoding logic lives here. Testable natively with `cargo test`.
- `wasm/` — Thin `wasm-bindgen` shell. Accepts JSON strings, calls core, returns results.

## Prerequisites

```bash
# Install wasm-pack
cargo install wasm-pack

# (Optional) Install wasm-opt for size optimisation
# macOS:
brew install binaryen
# or via cargo:
cargo install wasm-opt
```

## Build

### Native (tests only)

```bash
cargo test -p ootle-wasm-core
```

### WASM

```bash
# Build for bundler (webpack, vite, rollup, esbuild)
wasm-pack build crates/ootle_wasm/wasm --target bundler --release --out-dir ../../../pkg

# Build for Node.js
wasm-pack build crates/ootle_wasm/wasm --target nodejs --release --out-dir ../../../pkg

# Build for web (no bundler)
wasm-pack build crates/ootle_wasm/wasm --target web --release --out-dir ../../../pkg
```

The output goes to `pkg/` at the repo root containing:

- `ootle_wasm_bg.wasm` — the WASM binary
- `ootle_wasm.js` — JS glue code
- `ootle_wasm.d.ts` — TypeScript type definitions

### Size optimisation (optional)

```bash
wasm-opt -Oz --strip-debug --strip-producers pkg/ootle_wasm_bg.wasm -o pkg/ootle_wasm_bg.wasm
```

### Debug build (with panic hook)

```bash
wasm-pack build crates/ootle_wasm/wasm --target bundler --dev --out-dir ../../../pkg -- --features debug
```

## WASM Exports

| Function                  | Signature                                                                   | Description                                         |
|---------------------------|-----------------------------------------------------------------------------|-----------------------------------------------------|
| `borEncodeTransaction`    | `(transactionJson: string) → string`                                        | BOR-encode a Transaction JSON → base64 envelope     |
| `hashUnsignedTransaction` | `(unsignedTxJson: string, sealSignerPubKeyHex: string) → Uint8Array`        | Hash an unsigned transaction for signing (64 bytes) |
| `sr25519Sign`             | `(secretKeyHex: string, message: Uint8Array) → Uint8Array`                  | sr25519-sign a message (64-byte signature)          |
| `generateKeypair`         | `() → { secret_key, public_key }`                                           | Generate a random Ristretto keypair                 |
| `publicKeyFromSecretKey`  | `(secretKeyHex: string) → string`                                           | Derive public key from secret key                   |

All keys and signatures are lowercase hex-encoded strings.

## Usage from JS/TS

```typescript
import init, {
  generateKeypair,
  sr25519Sign,
  hashUnsignedTransaction,
  borEncodeTransaction,
  publicKeyFromSecretKey,
} from './pkg/ootle_wasm.js';

// Initialise the WASM module
await init();

// Generate a keypair
const { secret_key, public_key } = generateKeypair();

// Hash an unsigned transaction for signing
const unsignedTxJson = JSON.stringify(unsignedTransaction);
const hash = hashUnsignedTransaction(unsignedTxJson, sealSignerPublicKeyHex);

// Sign the hash
const signature = sr25519Sign(secret_key, hash);
```
