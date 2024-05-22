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

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    pub struct ResourceTest {
        fungible: Vault,
        non_fungible: Vault,
        confidential: Vault,
    }

    impl ResourceTest {
        pub fn new() -> Component<Self> {
            let fungible = ResourceBuilder::fungible().initial_supply(1000);
            let non_fungible = ResourceBuilder::non_fungible()
                .initial_supply([NonFungibleId::from_u64(1), NonFungibleId::from_u64(2)]);
            let confidential = ResourceBuilder::confidential()
                .mintable(AccessRule::AllowAll)
                .initial_supply(ConfidentialOutputStatement::mint_revealed(1000));

            Component::new(Self {
                fungible: Vault::from_bucket(fungible),
                non_fungible: Vault::from_bucket(non_fungible),
                confidential: Vault::from_bucket(confidential),
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        pub fn fungible_join(&self) {
            let b1 = self.fungible.withdraw(10);
            let b2 = self.fungible.withdraw(900);
            let joined = b1.join(b2);
            assert_eq!(joined.amount(), 910);
            self.fungible.deposit(joined);
        }

        pub fn non_fungible_join(&self) {
            let b1 = self.non_fungible.withdraw_non_fungible(NonFungibleId::from_u64(1));
            let b2 = self.non_fungible.withdraw_non_fungible(NonFungibleId::from_u64(2));
            let joined = b1.join(b2);
            assert_eq!(joined.amount(), 2);
            self.non_fungible.deposit(joined);
        }

        pub fn confidential_join(&self, output: ConfidentialOutputStatement) {
            let commitments = ResourceManager::get(self.confidential.resource_address()).mint_confidential(output);
            let b1 = self
                .confidential
                .withdraw_confidential(ConfidentialWithdrawProof::revealed_withdraw(10));
            let b2 = self
                .confidential
                .withdraw_confidential(ConfidentialWithdrawProof::revealed_withdraw(900));
            let joined = b1.join(b2);
            let joined = joined.join(commitments);
            assert_eq!(joined.amount(), 910);
            self.confidential.deposit(joined);
            assert_eq!(self.confidential.commitment_count(), 1);
        }
    }
}
