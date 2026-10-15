//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// pub mod eth_style;
pub mod committees;
pub mod epoch_checkpoints;
pub mod indexer_events;
pub mod misc;
pub mod network;
pub mod resources;
pub mod substates;
pub mod templates;
pub mod transaction_events;
pub mod transaction_receipts;
pub mod transactions;
pub mod utxos;
pub mod validators;
pub mod watched;

pub type HandlerResult<T> = Result<T, super::error::ErrorResponse>;
