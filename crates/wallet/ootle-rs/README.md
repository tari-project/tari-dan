# ootle.rs

[![Crates.io](https://img.shields.io/crates/v/ootle-rs.svg)](https://crates.io/crates/ootle-rs)
[![Documentation](https://docs.rs/ootle-rs/badge.svg)](https://docs.rs/ootle-rs)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-APACHE)

**High-performance, familiar interaction with the Tari Ootle (Layer 2) network.**

`ootle.rs` is a pure Rust library designed to be the standard interface for interacting with Tari Ootle. It is
architected to mirror the interface of [alloy-rs](https://github.com/alloy-rs/alloy), providing a seamless developer
experience for those transitioning from Ethereum or generic blockchain development to the Tari ecosystem.

## ✨ Features

* **Alloy-like API:** Uses the familiar `Provider`, `Signer`, and `Transport` architecture found in `alloy`.
* **Ootle Native:** First-class support for Tari Ootle
* **Type-Safe:** Strongly typed interactions with Tari's confidential assets.
* **Verified:** Substate reads and committed transactions are checked against proofs signed by the network's validators, so an indexer cannot forge them.
* **Confidential & Cross-Layer:** Stealth transfers and claiming Layer 1 (minotari) burns (`claim_burn::ClaimBurn`).
* **Async/Await:** Built on `tokio` for high-performance, non-blocking I/O.

## 📦 Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
ootle-rs = "0.10.0"
```

## 🚀 Quick Start

Connect to a local Ootle indexer, create a wallet, and send a transaction.

```rust
use ootle_rs::{
    address,
    builtin_templates::account::IAccount,
    key_provider::PrivateKeyProvider,
    provider::ProviderBuilder,
    wallet::OotleWallet,
    TransactionRequest,
};
use tari_ootle_common_types::Network;
use tari_template_lib_types::constants::TARI_TOKEN;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let network = Network::LocalNet;
    let indexer_api_url = "http://127.0.0.1:12500";

    // 1. Setup a wallet with a random key
    let signer = PrivateKeyProvider::random(network);
    let wallet = OotleWallet::from(signer);

    // 2. Create a provider
    let provider = ProviderBuilder::new()
        .with_network(network)
        .wallet(wallet)
        .connect(indexer_api_url)
        .await?;

    // 3. Craft and send a transaction (e.g., using IAccount for a transfer)
    let recipient = address!("otl_loc_10mc0v2lyy43kldl0ft4c2x5pe7j0ckduv8zej6jgr2z2g9m07fz7gl96ar5wwgu0qu0atmr5tl53ye7n38xr5u7ytlmudq0ruxcau0gge7rxk");
    
    let unsigned_tx = IAccount::new(&provider)
        .pay_fee(1000u64)
        .public_transfer(&recipient, TARI_TOKEN, 1_000_000u64)
        .prepare()
        .await?;

    let tx = TransactionRequest::default()
        .with_transaction(unsigned_tx)
        .build(provider.wallet())
        .await?;

    let pending_tx = provider.send_transaction(tx).await?;
    let outcome = pending_tx.watch().await?;

    println!("✅ Transaction confirmed: {:?}", outcome);

    Ok(())
}
```

## 🏗 Architecture

ootle.rs follows the modular design of alloy:

- Core: Defines the primitive types (Addresses, Signatures, Confidential Commitments) specific to Tari.
- Transport: Handles communication with Ootle Indexers and Validator Nodes (VNs).
- Provider: The high-level API for sending requests and managing state.
- Signer: Abstractions for signing transactions (support for Ristretto keys).

## 🤝 Contributing

We welcome contributions! Please see CONTRIBUTING.md for details on how to get started.

## 📄 License

This project is dual-licensed under either:
Apache License, Version 2.0, (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)
MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)
at your option.

---

"To the Ootle!" 🦉
