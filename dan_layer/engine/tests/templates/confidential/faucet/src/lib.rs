//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_template_lib::prelude::*;

#[template]
mod faucet_template {
    use super::*;

    pub struct ConfidentialFaucet {
        vault: Vault,
    }

    impl ConfidentialFaucet {
        pub fn mint(confidential_proof: ConfidentialOutputStatement) -> Component<Self> {
            let coins = ResourceBuilder::confidential()
                .mintable(AccessRule::AllowAll)
                .initial_supply(confidential_proof)
                .build_bucket();

            Component::new(Self {
                vault: Vault::from_bucket(coins),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn mint_with_view_key(
            confidential_proof: ConfidentialOutputStatement,
            view_key: RistrettoPublicKeyBytes,
        ) -> Component<Self> {
            let coins = ResourceBuilder::confidential()
                .mintable(AccessRule::AllowAll)
                .initial_supply(confidential_proof)
                .with_view_key(view_key)
                .build_bucket();

            Component::new(Self {
                vault: Vault::from_bucket(coins),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn mint_revealed(&mut self, amount: Amount) {
            let proof = ConfidentialOutputStatement::mint_revealed(amount);
            let bucket = ResourceManager::get(self.vault.resource_address()).mint_confidential(proof);
            self.vault.deposit(bucket);
        }

        pub fn mint_revealed_with_range_proof(&mut self, amount: Amount) {
            let mut proof = ConfidentialOutputStatement::mint_revealed(amount);
            proof.range_proof = vec![1, 2, 3];
            let bucket = ResourceManager::get(self.vault.resource_address()).mint_confidential(proof);
            self.vault.deposit(bucket);
        }

        pub fn mint_more(&mut self, proof: ConfidentialOutputStatement) {
            let bucket = ResourceManager::get(self.vault.resource_address()).mint_confidential(proof);
            self.vault.deposit(bucket);
        }

        pub fn take_free_coins(&mut self, proof: ConfidentialWithdrawProof) -> Bucket {
            debug!(
                "Withdrawing {} revealed coins from faucet and {} commitments",
                proof.revealed_input_amount(),
                proof.inputs.len()
            );
            self.vault.withdraw_confidential(proof)
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.vault.resource_address()).total_supply()
        }

        /// Utility function for tests
        pub fn split_coins(bucket: Bucket, proof: ConfidentialWithdrawProof) -> (Bucket, Bucket) {
            bucket.split_confidential(proof)
        }

        pub fn vault_balance(&self) -> Amount {
            self.vault.balance()
        }
    }
}
