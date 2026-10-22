# `ootle_ledger_common`

## Overview

`ootle_ledger_common` defines the APDU protocol spoken between the Tari Ootle Ledger device app
and a host client ([`ootle_ledger_client`](https://crates.io/crates/ootle_ledger_client)). Both
sides depend on this crate so the wire format — instruction set, status words, request/response
bodies, and the framing of the streamed `SignTransaction` exchange — is defined exactly once.

The crate is `no_std` by default so it can build inside the Ledger embedded app; hosts enable the
`std` feature. Request/response bodies are [borsh](https://crates.io/crates/borsh)-encoded.

## Protocol summary

All commands use APDU class byte `0x80`.

| Instruction       | Code   | Request body          | Response                  |
|-------------------|--------|-----------------------|---------------------------|
| `GetVersion`      | `0x01` | —                     | App version (UTF-8)       |
| `GetAppName`      | `0x02` | —                     | App name (UTF-8)          |
| `GetPublicKey`    | `0x03` | `GetPublicKeyRequest` | `GetPublicKeyResponse`    |
| `SignTransaction` | `0x04` | Streamed (see below)  | `SignTransactionResponse` |

### `GetPublicKey`

`GetPublicKeyRequest` names a derivation path and may carry a `StealthTweak` (network + the spent
UTXO's sender public nonce `R`). With the tweak the device returns the stealth one-time key
`(c + k)·G`, where `c = H(v·R)` and `v` is the view key it derives at the same path — the key it also
signs with when the same nonce is given in a `SignTransaction` header — so a host
can resolve the seal key of a stealth input before binding any signature to it.

### `SignTransaction` streaming

A signing exchange streams the canonical transaction signing preimage to the device as a sequence
of frames, distinguished by `P2` (`FrameKind`):

1. one `Header` frame carrying a `SignTransactionHeader` (key-derivation path, signing mode, and
   optional stealth nonce),
2. one `Segment` frame per preimage field (`SigningField`, tagged in the low 7 bits of `P1`), in
   chain order, each possibly split across multiple APDUs with the high bit of `P1` marking the
   field's last chunk,
3. one `Finalize` frame, which triggers the user review on-device and, on approval, returns the
   public key and Schnorr signature.

The device recomputes the transaction message digest and Schnorr challenge itself from the
streamed bytes using the domain-separation constants in the `signing` module, so a compromised
host cannot substitute a different message than the one the user reviews.

Errors are reported as app-specific status words (`OotleStatusWord`) offset into the `0xB0xx`
range.

## Documentation

Detailed documentation is available at
[docs.rs/ootle_ledger_common](https://docs.rs/ootle_ledger_common).
