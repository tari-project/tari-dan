# ootle-wasm

Client-side WebAssembly crypto for [Tari Ootle](https://www.tari.com/) L2. Handles BOR encoding, transaction hashing, Schnorr signing, and key management — with output byte-identical to the native Rust implementation.

## Installation

```bash
npm install @tari-project/ootle-wasm
```

## API

All keys and signatures are **`Uint8Array`** (raw 32-byte values).

### `generateKeypair()`

Generate a new random Ristretto255 keypair.

```typescript
import { generateKeypair } from "@tari-project/ootle-wasm";

const { secret_key, public_key } = generateKeypair();
// secret_key: Uint8Array (32 bytes)
// public_key: Uint8Array (32 bytes)
```

### `publicKeyFromSecretKey(secretKey)`

Derive the public key from a secret key.

```typescript
import { publicKeyFromSecretKey } from "@tari-project/ootle-wasm";

const publicKey = publicKeyFromSecretKey(secretKey);
// publicKey: Uint8Array (32 bytes)
```

### `hashUnsignedTransaction(unsignedTxJson, sealSignerPublicKey)`

Hash an `UnsignedTransactionV1` for signing. Returns a 64-byte `Uint8Array` that should be passed to `schnorrSign`.

- `unsignedTxJson` — JSON-serialised `UnsignedTransactionV1`
- `sealSignerPublicKey` — raw public key bytes of the account owner (seal signer)

```typescript
import { hashUnsignedTransaction } from "@tari-project/ootle-wasm";

const hash = hashUnsignedTransaction(
  JSON.stringify(unsignedTransaction),
  sealSignerPublicKey,
);
```

### `schnorrSign(secretKey, message)`

Schnorr-sign a message (typically the hash from `hashUnsignedTransaction`).

```typescript
import { schnorrSign } from "@tari-project/ootle-wasm";

const { public_nonce, signature } = schnorrSign(secretKey, hash);
// public_nonce: Uint8Array (32 bytes)
// signature:    Uint8Array (32 bytes)
```

## External signers (HSM, MPC)

These functions sign a transaction without passing a secret key to the binding. The external signer signs a 64-byte hash and returns a raw 64-byte signature (`R ‖ s`); the binding verifies it and attaches it.

### `hashUnsealedTransaction(txJson)`

Hash a transaction for sealing. Returns the 64-byte seal message for the seal signer (the account owner). Accepts unsigned or unsealed transaction JSON. The seal message covers every authorization signature, so attach them all before hashing.

### `attachSealSignature(txJson, sealSignerPublicKey, signature)`

Seal a transaction with the seal signer's signature over the `hashUnsealedTransaction` hash. Returns the sealed `Transaction` JSON. Throws if the seal signature, or any authorization signature already attached, does not verify.

### `attachTransactionSignature(txJson, signerPublicKey, signature, sealSignerPublicKey)`

Append an authorization signature from an additional authorizer key. The signature is over the `hashUnsignedTransaction` hash for `sealSignerPublicKey`. Returns the unsealed transaction JSON. Throws if the signature does not verify.

```typescript
import {
  hashUnsignedTransaction,
  hashUnsealedTransaction,
  attachTransactionSignature,
  attachSealSignature,
  borEncodeTransaction,
} from "@tari-project/ootle-wasm";

let txJson = JSON.stringify(unsignedTransaction);

// Optional: authorizations from other keys
const authHash = hashUnsignedTransaction(txJson, sealSignerPublicKey);
const authSig = await externalSigner.sign(authorizerKeyId, authHash);
txJson = attachTransactionSignature(txJson, authorizerPublicKey, authSig, sealSignerPublicKey);

// Seal last
const sealHash = hashUnsealedTransaction(txJson);
const sealSig = await externalSigner.sign(sealSignerKeyId, sealHash);
const transactionJson = attachSealSignature(txJson, sealSignerPublicKey, sealSig);

const envelope = borEncodeTransaction(transactionJson);
```

### `borEncodeTransaction(transactionJson)`

BOR-encode a signed `Transaction` into a base64 `TransactionEnvelope` string, ready to submit to the network.

```typescript
import { borEncodeTransaction } from "@tari-project/ootle-wasm";

const envelope = borEncodeTransaction(JSON.stringify(transaction));
```

## Working with keys

Keys and signatures are raw `Uint8Array` bytes. Convert to and from hex strings using the built-in methods (Node 22+, modern browsers):

```typescript
import { generateKeypair, publicKeyFromSecretKey } from "@tari-project/ootle-wasm";

// Generate a keypair and display as hex
const { secret_key, public_key } = generateKeypair();
console.log("Public key:", public_key.toHex());

// Load a key from a hex string
const restored = Uint8Array.fromHex("a1b2c3...");
const derivedPublicKey = publicKeyFromSecretKey(restored);
```

## Full example

```typescript
import {
  generateKeypair,
  publicKeyFromSecretKey,
  hashUnsignedTransaction,
  schnorrSign,
  borEncodeTransaction,
} from "@tari-project/ootle-wasm";

// 1. Generate or load a keypair
const { secret_key, public_key } = generateKeypair();

// 2. Build an unsigned transaction (application-specific)
const unsignedTx = {
  /* ... UnsignedTransactionV1 fields ... */
};

// 3. Hash for signing
const hash = hashUnsignedTransaction(JSON.stringify(unsignedTx), public_key);

// 4. Sign
const { public_nonce, signature } = schnorrSign(secret_key, hash);

// 5. Assemble the signed transaction and BOR-encode
const signedTx = {
  /* ... Transaction with signature attached ... */
};
const envelope = borEncodeTransaction(JSON.stringify(signedTx));

// 6. Submit envelope to the Ootle network
```

## License

BSD-3-Clause
