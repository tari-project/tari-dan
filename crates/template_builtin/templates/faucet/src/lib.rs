//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::{prelude::*, types::constants::XTR_FAUCET_CLAIM_RESOURCE_ADDRESS};

#[template]
mod template {
    /// Faucet amount in microtari: 1,000 TARI per claim. Fixed for take(), ceiling for take_confidential().
    const FAUCET_AMOUNT: u64 = 1_000 * 1_000_000;
    use super::*;

    pub struct XtrFaucet {
        vault: Vault,
    }

    impl XtrFaucet {
        /// Mints a claim-receipt NFT keyed to the caller's public key, then immediately burns it.
        /// The burned substate key persists on-chain, so a second attempt panics with DuplicateNonFungibleId.
        fn record_claim(&self, address: ComponentAddress) {
            let receipt = ResourceManager::get(XTR_FAUCET_CLAIM_RESOURCE_ADDRESS).mint_non_fungible(
                NonFungibleId::from_u256(address.as_object_key().into_array()),
                &(),
                &(),
            );
            receipt.burn();
        }

        /// Gives exactly 1,000 TARI to the caller. Can only be called once per component.
        pub fn take(&self, component: ComponentManager) {
            self.record_claim(component.component_address());
            emit_event("take", metadata!["amount" => FAUCET_AMOUNT.to_string()]);
            let bucket = self.vault.withdraw(FAUCET_AMOUNT);
            component.invoke("deposit", args!(bucket));
        }

        /// Gives exactly 1,000 TARI to the caller using a proof to authorize the deposit. Can only be called once per
        /// component.
        pub fn take_with_proof(&self, proof: Proof, component: ComponentManager) {
            self.record_claim(component.component_address());
            emit_event("take", metadata!["amount" => FAUCET_AMOUNT.to_string()]);
            let bucket = self.vault.withdraw(FAUCET_AMOUNT);
            proof.authorize_with(|| {
                component.invoke("deposit", args!(bucket));
            })
        }

        /// Tops up the faucet vault. Permissionless: any caller may return or add funds. A bucket of any
        /// resource other than the faucet's own is rejected by the vault.
        pub fn deposit(&self, bucket: Bucket) {
            emit_event("deposit", metadata!["amount" => bucket.amount().to_string()]);
            self.vault.deposit(bucket);
        }
    }
}
