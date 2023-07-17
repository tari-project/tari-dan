//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_engine_types::substate::{Substate, SubstateAddress};
use tari_transaction::Transaction;

#[async_trait]
pub trait SubstateResolver {
    type Error: Send + Sync + 'static;

    async fn resolve<T: Extend<(SubstateAddress, Substate)> + Send>(
        &self,
        transaction: &Transaction,
        out: &mut T,
    ) -> Result<(), Self::Error>;
}
