# tari_ootle_transaction

[![Crates.io](https://img.shields.io/crates/v/tari_ootle_transaction.svg)](https://crates.io/crates/tari_ootle_transaction)
[![Documentation](https://docs.rs/tari_ootle_transaction/badge.svg)](https://docs.rs/tari_ootle_transaction)

Transaction builder and data types for the Tari Ootle layer-2 protocol.
Provides a fluent builder for constructing multi-instruction transactions,
a workspace abstraction for passing outputs between instructions within a
transaction, and the signed/unsigned/unsealed transaction types consumed by
the engine and validator nodes.

## Key types

| Type                  | Description                                                  |
|-----------------------|--------------------------------------------------------------|
| `Transaction`         | A fully signed, sealed transaction ready for submission      |
| `UnsealedTransaction` | A built transaction pending the seal (main signer) signature |
| `TransactionBuilder`  | Fluent builder for constructing transactions                 |
| `Instruction`         | A single operation: call, deposit, withdraw, assert, …       |
| `TransactionId`       | Unique identifier derived from the transaction hash          |

## Example

```rust
use tari_ootle_transaction::{Transaction, args, Network};
use tari_template_lib::types::constants::TARI_TOKEN;

// All transactions start from Transaction::builder(network) or the
// convenience alias Transaction::builder_localnet().
let tx = Transaction::builder(Network::Esmeralda)
// Pay fees from the sender account (max 1000 units)
.pay_fee_from_component(sender_account, 1000)
// Withdraw TARI from the sender's account
.call_method(sender_account, "withdraw", args![TARI_TOKEN, 500u64])
.put_last_instruction_output_on_workspace("bucket")
// Deposit into the receiver's account
.call_method(receiver_account, "deposit", args![Workspace("bucket")])
// Finalise the instruction set — produces an UnsealedTransaction
.finish()
// Sign with the sender's secret key to produce a Transaction
.seal( & sender_secret_key);
```

The `args!` macro encodes method arguments; `Workspace("label")` refers to
the output of a previous instruction that was placed on the workspace with
`put_last_instruction_output_on_workspace`.

## License

BSD-3-Clause. Copyright 2022 The Tari Project.
