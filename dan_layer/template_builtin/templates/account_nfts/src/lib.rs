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
mod account_non_fungible_template {
    use super::*;

    pub struct AccountNonFungible {
        resource_address: ResourceAddress,
    }

    impl AccountNonFungible {
        pub fn create(owner_token: NonFungibleAddress, token_symbol: String) -> AccountNonFungibleComponent {
            // extract the public key from the token
            // we only allow owner tokens that correspond to public keys
            let public_key = owner_token
                .to_public_key()
                .unwrap_or_else(|| panic!("owner_token is not a valid public key: {}", owner_token));
            // the account component will be addressed using the public key
            let component_id = public_key.as_hash();

            // create the resource address
            let resource_address = ResourceBuilder::non_fungible(token_symbol).build();

            // only the owner of the token will be able to withdraw funds from the account
            let mint_rule = AccessRule::Restricted(Require(owner_token));
            let rules = AccessRules::new()
                .add_method_rule("get_resource_address", AccessRule::AllowAll)
                .default(mint_rule);

            Self { resource_address }.create_with_options(rules, Some(component_id))
        }

        pub fn mint(&mut self, metadata: Metadata) -> Bucket {
            // Mint a new token with a random ID
            let id = NonFungibleId::random();
            self.mint_specific(id, metadata)
        }

        pub fn mint_specific(&mut self, id: NonFungibleId, metadata: Metadata) -> Bucket {
            emit_event(
                "mint",
                [
                    ("id".to_string(), id.to_string()),
                    ("metadata".to_string(), metadata.to_string()),
                    ("resource_address".to_string(), self.resource_address.to_string()),
                ],
            );

            // Mint the NFT, this will fail if the token ID already exists
            ResourceManager::get(self.resource_address).mint_non_fungible(id, &metadata, &{})
        }

        pub fn get_resource_address(&self) -> ResourceAddress {
            self.resource_address
        }
    }
}
