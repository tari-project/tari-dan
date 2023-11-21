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

    pub struct TestFaucet {
        vault: Vault,
    }

    impl TestFaucet {
        pub fn mint(initial_supply: Amount) -> Component<Self> {
            Self::mint_with_symbol(initial_supply, "faucets".to_string())
        }

        pub fn mint_with_symbol(initial_supply: Amount, symbol: String) -> Component<Self> {
            let coins = ResourceBuilder::fungible()
                .with_token_symbol(symbol)
                .initial_supply(initial_supply)
                .build_bucket();

            Component::new(Self {
                vault: Vault::from_bucket(coins),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn take_free_coins(&mut self) -> Bucket {
            debug!("Withdrawing 1000 coins from faucet");
            self.vault.withdraw(Amount(1000))
        }

        pub fn take_free_coins_confidential(&mut self, proof: ConfidentialWithdrawProof) -> Bucket {
            debug!("Withdrawing <unknown> coins from faucet");
            self.vault.withdraw_confidential(proof)
        }

        pub fn burn_coins(&mut self, amount: Amount) {
            let mut bucket = self.vault.withdraw(amount);
            bucket.burn();
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.vault.resource_address()).total_supply()
        }

        pub fn pay_fee(&mut self, amount: Amount) {
            debug!("Paying fee from faucet");
            self.vault.pay_fee(amount);
        }

        pub fn pay_fee_confidential(&mut self, proof: ConfidentialWithdrawProof) {
            debug!("Paying fee from faucet");
            self.vault.pay_fee_confidential(proof);
        }
    }
}
