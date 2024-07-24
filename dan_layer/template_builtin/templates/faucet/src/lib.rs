//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use tari_template_lib::crypto::BalanceProofSignature;

    use super::*;

    pub struct XtrFaucet {
        vault: Vault,
    }

    impl XtrFaucet {
        pub fn take(&self, amount: Amount) -> Bucket {
            debug!("Withdrawing {} coins from faucet", amount);
            self.vault.withdraw(amount)
        }

        pub fn take_confidential(
            &self,
            amount: Amount,
            output: ConfidentialOutputStatement,
            balance_proof: BalanceProofSignature,
        ) -> Bucket {
            // Withdraws revealed funds into the given confidential output
            let proof = ConfidentialWithdrawProof::revealed_to_confidential(amount, output, balance_proof);
            debug!("Withdrawing {} coins from faucet into confidential output", amount);
            self.vault.withdraw_confidential(proof)
        }
    }
}
