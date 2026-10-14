//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod account;
mod address_book_entry;
mod api_key;
mod authored_template;
mod balance_change;
mod confidential_output;
mod config;
mod epoch_birthday;
mod event;
pub mod input_selection;
mod key;
mod lock;
mod lock_guard;
mod non_fungible_tokens;
mod resource;
mod signing_request;
mod stealth_output;
mod substate;
mod transaction_request;
mod utxo_update;
mod vault;
mod wallet_transaction;
mod webauthn_registration;

pub use account::*;
pub use address_book_entry::*;
pub use api_key::*;
pub use authored_template::*;
pub use balance_change::*;
pub use confidential_output::*;
pub use config::Config;
pub use epoch_birthday::*;
pub use event::*;
pub use key::*;
pub use lock::*;
pub use lock_guard::*;
pub use non_fungible_tokens::*;
pub use resource::*;
pub use signing_request::*;
pub use stealth_output::*;
pub use substate::*;
pub use transaction_request::*;
pub use utxo_update::*;
pub use vault::*;
pub use wallet_transaction::*;
pub use webauthn_registration::*;

pub type WalletLockId = i32;
