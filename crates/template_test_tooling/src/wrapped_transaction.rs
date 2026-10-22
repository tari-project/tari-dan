//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine::executables::{Executable, Instructions, WeightedExecutable};
use tari_ootle_common_types::InputDeclaration;
use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};

pub struct WrappedTransaction {
    transaction: tari_ootle_transaction::Transaction,
    inputs: Vec<InputDeclaration>,
}

impl WrappedTransaction {
    pub fn new(transaction: tari_ootle_transaction::Transaction) -> Self {
        Self {
            transaction,
            inputs: vec![],
        }
    }

    pub fn extend_inputs<I: IntoIterator<Item = InputDeclaration>>(&mut self, inputs: I) -> &mut Self {
        self.inputs.extend(inputs);
        self
    }
}

impl Executable for WrappedTransaction {
    fn to_id(&self) -> tari_ootle_transaction::TransactionId {
        self.transaction.calculate_id()
    }

    fn calculate_intent_commitment(&self) -> Hash32 {
        self.transaction.calculate_intent_commitment()
    }

    fn to_id_and_intent_commitment(&self) -> (tari_ootle_transaction::TransactionId, Hash32) {
        self.transaction.to_id_and_intent_commitment()
    }

    fn all_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_ {
        // The transaction's own declarations win: the extra inputs exist so a test need not declare
        // every substate, not to overrule a test that declared one deliberately.
        let extra = self.inputs.iter().filter(|extra| {
            !self
                .transaction
                .all_inputs_iter()
                .any(|declared| declared.substate_id() == extra.substate_id())
        });
        self.transaction
            .all_inputs_iter()
            .map(|decl| decl.to_owned())
            .chain(extra.cloned())
    }

    fn charged_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_ {
        self.transaction.all_inputs_iter().map(|decl| decl.to_owned())
    }

    fn main_signer(&self) -> Option<RistrettoPublicKeyBytes> {
        self.transaction.main_signer()
    }

    fn signers_iter(&self) -> impl Iterator<Item = &RistrettoPublicKeyBytes> {
        self.transaction.signers_iter()
    }

    fn into_instructions(self) -> Instructions {
        self.transaction.into_instructions()
    }
}

impl WeightedExecutable for WrappedTransaction {
    fn calculate_weight(&self) -> tari_ootle_transaction::TransactionWeight {
        self.transaction.calculate_weight()
    }
}
