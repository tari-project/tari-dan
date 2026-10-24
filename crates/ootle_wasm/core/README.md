# ootle-wasm-core

Pure Rust library implementing the crypto and encoding operations needed by Tari Ootle WASM clients. This crate has no WASM dependencies and can be used natively from any Rust project.

## Modules

### `bor` — BOR (CBOR) encoding

Encodes `Transaction` values using `tari_bor` and returns base64 strings matching the `TransactionEnvelope` format used by the Ootle network.

- `bor_encode_transaction(&Transaction) → Result<String>`
- `bor_encode_transaction_json(json: &str) → Result<String>`

### `hash` — Transaction hashing

Produces the 64-byte signing messages. Delegates to `TransactionSignature::create_message_v1` (authorization) and `TransactionSealSignature::create_message_v1` (seal) to guarantee byte-identical output with the rest of the Tari codebase.

- `hash_unsigned_transaction(&UnsignedTransactionV1, seal_signer_hex: &str) → Result<Vec<u8>>`
- `hash_unsigned_transaction_json(json: &str, seal_signer_hex: &str) → Result<Vec<u8>>`
- `hash_unsealed_transaction(&UnsealedTransactionV1) → Vec<u8>`: seal message; covers every authorization signature, so attach them first
- `hash_unsealed_transaction_json(json: &str) → Result<Vec<u8>>`: accepts unsigned or unsealed JSON

### `transaction` — Sealing and authorization

Each function accepts unsigned or unsealed transaction JSON.

- `seal_transaction_json(json, seal_signer_secret_key) → Result<String>`
- `add_transaction_signer_json(json, signer_secret_key, seal_signer_public_key) → Result<String>`
- `attach_seal_signature_json(json, seal_signer_public_key, signature) → Result<String>`: seals with an externally produced 64-byte `R ‖ s` signature over the seal message
- `attach_transaction_signature_json(json, signer_public_key, signature, seal_signer_public_key) → Result<String>`: appends an externally produced authorization signature

The `attach_*` functions verify the signature before returning, so a signature over the wrong message or by the wrong key fails at attach time.

### `sign` — Schnorr signing and key management

Ristretto255 Schnorr signatures and keypair operations.

- `schnorr_sign(secret_key_hex: &str, message: &[u8]) → Result<SchnorrSignatureResult>`
- `public_key_from_secret_key(secret_key_hex: &str) → Result<String>`
- `generate_keypair() → KeypairResult`

### `error` — Error types

`OotleWasmError` covers JSON, BOR, hex, and key-related failures.

## Testing

```bash
cargo test -p ootle-wasm-core
```

Tests verify that hashing output is identical to `tari_ootle_transaction`, that JSON round-trips produce the same hashes, and that keypair generation and signing work correctly.
