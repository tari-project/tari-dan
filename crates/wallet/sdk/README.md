# tari_ootle_wallet_sdk

[![crates.io](https://img.shields.io/crates/v/tari_ootle_wallet_sdk)](https://crates.io/crates/tari_ootle_wallet_sdk)
[![docs.rs](https://docs.rs/tari_ootle_wallet_sdk/badge.svg)](https://docs.rs/tari_ootle_wallet_sdk)

An opinionated, batteries-included wallet SDK for the Tari Ootle network. Provides account management, key derivation,
transaction building/submission, stealth transfers, and confidential transactions.

This crate is intended for building full-featured wallet applications (like `tari_walletd`). If you are building a
lighter-weight project or want more control over the components you use, consider
[ootle-rs](https://crates.io/crates/ootle-rs) instead.

## Architecture

The SDK is generic over `WalletSdkSpec`, which defines pluggable backends for storage, key management, and network
access:

```rust
use tari_ootle_wallet_sdk::{WalletSdk, WalletSdkSpec};

// WalletSdkSpec requires:
//   Store: WalletStore              — persistent storage backend
//   KeyStore: WalletKeyStore        — cryptographic key management
//   NetworkInterface: WalletNetworkInterface — network communication (indexer)
```

Once initialized, domain-specific APIs are accessed via accessor methods on `WalletSdk`:

| API                           | Description                                       |
|-------------------------------|---------------------------------------------------|
| `accounts_api()`              | Create, list, rename, and query accounts          |
| `key_manager_api()`           | Derive keys and account addresses                 |
| `transaction_api()`           | Build, sign, submit, and track transactions       |
| `confidential_transfer_api()` | Confidential (account and vault-based) transfers  |
| `stealth_transfer_api()`      | Stealth transfers                                 |
| `stealth_outputs_api()`       | Query unspent stealth UTXOs                       |
| `substate_api()`              | Fetch substates from the network                  |
| `signer_api()`                | Sign transactions and arbitrary messages          |
| `non_fungible_api()`          | NFT management                                    |
| `template_api()`              | Template lookups                                  |
| `config_api()`                | Wallet configuration (network, indexer URL, etc.) |

## Usage

```rust,ignore
use tari_ootle_wallet_sdk::{WalletSdk, apis::transaction::Submission};

// Initialize the SDK (typically done once at startup)
let sdk = WalletSdk::initialize(config, store, key_store, network_interface)?;

// Derive a new account address and create an account
let address = sdk.key_manager_api().next_account_address()?;
let account = sdk.accounts_api().create_account(
    Some("my-account"),
    true, // set as default
    address,
)?;

// List accounts
let accounts = sdk.accounts_api().get_many(0, 10)?;
let total = sdk.accounts_api().count()?;

// Get account by name
let account = sdk.accounts_api().get_account_by_name("my-account")?;

// Query vault balances for an account
let vaults = sdk.accounts_api().get_vaults_by_account(account.component_address())?;

// Query unspent stealth outputs
let stealth = sdk.stealth_outputs_api()
    .get_unspent_outputs_by_account(account.component_address(), false)?;

// Submit a transaction. A wallet that verifies its indexer holds the transaction back until it trusts a
// validator set, and leaves submitting it again to the caller.
let tx_id = sdk.transaction_api().insert_new_transaction(transaction, None, &[], false)?;
loop {
    match sdk.transaction_api().submit_transaction(tx_id).await? {
        Submission::Submitted => break,
        Submission::Rejected => return Err(anyhow!("transaction {tx_id} was rejected")),
        Submission::Deferred => tokio::time::sleep(Duration::from_secs(5)).await,
    }
}
// None until the transaction finalizes
let finalized = sdk.transaction_api().check_and_store_finalized_transaction(tx_id).await?;
```

## Features

- **`ts`** — Generate TypeScript type bindings via `ts-rs`

## License

BSD-3-Clause
