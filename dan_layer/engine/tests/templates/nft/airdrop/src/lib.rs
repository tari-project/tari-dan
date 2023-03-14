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
use tari_template_abi::rust::collections::HashSet;
use tari_template_lib::prelude::*;

#[template]
mod airdrop_template {
    use super::*;

    pub struct Airdrop {
        allow_list: HashSet<ComponentAddress>,
        is_airdrop_open: bool,
        claimed_count: u32,
        vault: Vault,
    }

    impl Airdrop {
        pub fn new() -> Self {
            let bucket = ResourceBuilder::non_fungible()
                .with_token_symbol("AIR")
                .mint_many_with(1..=10, |n| (NonFungibleId::from_u32(n), (Vec::new(), Vec::new())))
                .build_bucket();

            Self {
                allow_list: HashSet::new(),
                is_airdrop_open: false,
                claimed_count: 0,
                vault: Vault::from_bucket(bucket),
            }
        }

        pub fn add_recipient(&mut self, address: ComponentAddress) {
            assert!(self.is_airdrop_open, "Airdrop already started");
            assert!(self.allow_list.len() < 10, "Airdrop allow list is full");
            assert!(!self.allow_list.contains(&address), "Address already in allow list");
            self.allow_list.insert(address);
        }

        pub fn open_airdrop(&mut self) {
            assert!(!self.is_airdrop_open, "Airdrop already open");
            self.is_airdrop_open = true;
        }

        pub fn claim_any(&mut self, address: ComponentAddress) -> Bucket {
            assert!(self.is_airdrop_open, "Airdrop is not open");
            // Note: this does not enforce that the token is deposited in an address from the allow list
            assert!(
                self.allow_list.remove(&address),
                "Address {} is not in allow list or has already been claimed",
                address
            );

            self.claimed_count += 1;
            self.vault.withdraw(Amount(1))
        }

        pub fn claim_specific(&mut self, address: ComponentAddress, id: NonFungibleId) -> Bucket {
            assert!(self.is_airdrop_open, "Airdrop is not open");
            assert!(
                self.allow_list.remove(&address),
                "Address {} is not in allow list or has already been claimed",
                address
            );

            self.claimed_count += 1;
            self.vault.withdraw_non_fungibles(Some(id))
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.vault.resource_address()).total_supply()
        }

        pub fn num_claimed(&self) -> u32 {
            self.claimed_count
        }

        pub fn vault_balance(&self) -> Amount {
            self.vault.balance()
        }

        // pub fn do_airdrop(&mut self) {
        //     assert!(!self.is_airdrop_done, "Airdrop already done");
        //     assert!(!self.allow_list.is_empty(), "Allow list is empty");
        //
        //     for (i, account_address) in self.allow_list.drain(..).enumerate() {
        //         let bucket = self
        //             .vault
        //             .withdraw_non_fungible(self.address, NonFungibleId::from_u32(i as u32));
        //
        //         TODO: Cross-template calls are not yet supported
        //         system().invoke_component::<()>(account_address, "deposit", args![bucket]);
        //     }
        //
        //     self.is_airdrop_done = true;
        // }
    }
}
