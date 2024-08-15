//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::{
    consensus_models::{SubstatePledges, TransactionRecord},
    StateStoreReadTransaction,
};

use crate::hotstuff::HotStuffError;

#[derive(Debug, Clone)]
pub struct PledgedTransaction {
    pub transaction: TransactionRecord,
    pub foreign_pledges: SubstatePledges,
    pub local_pledges: SubstatePledges,
}

impl PledgedTransaction {
    #[allow(clippy::mutable_key_type)]
    fn new(
        prepared_transaction: TransactionRecord,
        local_pledges: SubstatePledges,
        foreign_pledges: SubstatePledges,
    ) -> Self {
        Self {
            transaction: prepared_transaction,
            foreign_pledges,
            local_pledges,
        }
    }
}

impl PledgedTransaction {
    pub fn load_pledges<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        transaction: TransactionRecord,
    ) -> Result<PledgedTransaction, HotStuffError> {
        #[allow(clippy::mutable_key_type)]
        let local_pledges = transaction.get_local_pledges(tx)?;
        #[allow(clippy::mutable_key_type)]
        let foreign_pledges = transaction.get_foreign_pledges(tx)?;

        Ok(PledgedTransaction::new(transaction, local_pledges, foreign_pledges))
    }
}
