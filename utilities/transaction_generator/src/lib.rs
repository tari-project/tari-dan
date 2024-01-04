//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod transaction_builders;
mod transaction_reader;
mod transaction_writer;

pub use transaction_reader::*;
pub use transaction_writer::*;

pub type BoxedTransactionBuilder = Box<dyn Fn(u64) -> tari_transaction::Transaction + Send + Sync + 'static>;
