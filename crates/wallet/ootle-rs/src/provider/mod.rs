//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Network provider for interacting with the Ootle indexer.
//!
//! The provider is the main entry point for sending transactions, querying balances,
//! resolving transaction inputs, and streaming events.
//!
//! Use [`ProviderBuilder`] to connect to an indexer:
//!
//! ```rust,ignore
//! let provider = ProviderBuilder::new()
//!     .wallet(wallet)
//!     .connect("http://127.0.0.1:12500")
//!     .await?;
//! ```
//!
//! The [`Provider`] trait defines the core interface (network info, input resolution,
//! substate fetching), while [`WalletProvider`] extends it with wallet access for
//! signing and submitting transactions.

mod balance;
mod builder;
mod error;
mod event_watcher;
mod indexer;
mod input_resolver;
mod traits;
mod tx_stream;
mod tx_watcher;
mod utxo_watcher;
mod verifier;
mod want_input;

#[cfg(test)]
mod test_indexer;
#[cfg(test)]
mod test_sse_server;

pub use balance::*;
pub use builder::*;
pub use error::*;
pub use event_watcher::*;
pub use indexer::*;
pub use tari_ootle_light_client::TrustedValidatorSet;
pub use traits::*;
pub use tx_watcher::*;
pub use utxo_watcher::*;
pub use verifier::{IndexerTrust, SavedValidatorSet, VerifyError};
pub use want_input::*;
