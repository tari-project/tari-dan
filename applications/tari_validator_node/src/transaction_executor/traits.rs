//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::consensus_models::{ExecutedTransaction, Transaction};

pub trait TransactionExecutable {
    type Error: Send + Sync + 'static;

    fn execute(&self, transaction: Transaction) -> Result<ExecutedTransaction, Self::Error>;
}
