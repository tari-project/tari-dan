//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_template_abi::rust::collections::BTreeMap;
use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    pub struct Fungible {
        fungible: Vault,
        confidential: Vault,
    }

    impl Fungible {
        pub fn with_supply(supply: Amount) -> Component<Self> {
            let alloc = CallerContext::allocate_component_address(None);
            Self::with_address_and_supply(alloc, supply)
        }

        pub fn with_address_and_supply(alloc: ComponentAddressAllocation, supply: Amount) -> Component<Self> {
            let fungible = ResourceBuilder::public_fungible()
                .mintable(rule!(allow_all), OWNER)
                .initial_supply(supply);
            // Supply tracking is off so that the tests can mint past `Amount::MAX` in total and exercise the
            // vault's own overflow handling.
            let confidential = ResourceBuilder::confidential()
                .mintable(rule!(allow_all), OWNER)
                .disable_total_supply_tracking()
                .initial_supply(ConfidentialOutputStatement::mint_revealed(supply));

            Component::new(Self {
                fungible: Vault::from_bucket(fungible),
                confidential: Vault::from_bucket(confidential),
            })
            .with_address_allocation(alloc)
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn fungible_mint_more(&self, amount: Amount) {
            let bucket = ResourceManager::get(self.fungible.resource_address()).mint_fungible(amount);
            self.fungible.deposit(bucket);
        }

        pub fn confidential_mint_more(&self, output: ConfidentialOutputStatement) {
            let commitments = ResourceManager::get(self.confidential.resource_address()).mint_confidential(output, BTreeMap::new());
            self.confidential.deposit(commitments);
        }

        // Withdraws confidential/revealed tokens from the vault and deposits the resulting outputs back into the vault
        // according to the proof.
        pub fn convert(&self, proof: ConfidentialWithdrawProof) {
            let bucket = self.confidential.withdraw_confidential(proof);
            self.confidential.deposit(bucket);
        }

        pub fn create_confidential_proof_by_amount(&self, amount: Amount) -> Proof {
            self.confidential.create_proof_by_amount(amount)
        }

        pub fn create_confidential_proof(&self) -> Proof {
            self.confidential.create_proof()
        }

        pub fn confidential_balance(&self) -> Amount {
            self.confidential.balance()
        }

        pub fn fungible_withdraw(&self, amount: Amount) -> Bucket {
            self.fungible.withdraw(amount)
        }
    }
}
