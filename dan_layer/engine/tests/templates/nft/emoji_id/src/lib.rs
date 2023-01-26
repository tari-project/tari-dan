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

#[derive(Debug, Clone, Encode, Decode)]
pub enum Emoji {
    Smile,
    Sweat,
    Laugh,
    Wink,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct EmojiId {
    pub emojis: Vec<Emoji>,
}

/// This template implements a classic, first come first served, constant price NFT drop
#[template]
mod emoji_id {
    use super::*;  
    
    pub struct EmojiIdMinter {
        max_emoji_id_len: usize,
        mint_price: Amount,
        resource_address: ResourceAddress,
        earnings: Vault<Thaum>,
    }

    impl EmojiIdMinter {
        pub fn new(token_symbol: String, max_emoji_id_len: usize, mint_price: Amount) -> Self {
            // Create the non-fungible resource with empty initial supply
            let resource_address = ResourceBuilder::non_fungible()
                .with_token_symbol(token_symbol)
                .build();

            // TODO: how do you initialize a Thaum vault? Could it be similar with non-Thaum fungible resources?    
            let earnings = Vault::new_empty::<Thaum>();

            Self {
                max_emoji_id_len,
                mint_price,
                resource_address,
                earnings,
            }
        }

        // TODO: how do we ensure that the payment is indeed in Thaums?
        pub fn mint(&mut self, emojis: Vec<Emoji>, payment: Bucket) -> (Bucket, Bucket<Thaum>) {
            assert!(
                !emojis.empty() && emojis.len() <= self.max_emoji_id_len,
                "Invalid Emoji ID lenght"
            );

            // process the payment
            // no need to manually check the amount, as the split operation will fail if not enough funds
            let (cost, change) = payment.split(self.mint_price);
            self.earnings.deposit(cost);

            // mint a new emoji id
            // TODO: how do we ensure uniqueness of emoji ids? Two options:
            //      1. Derive the nft id from the emojis
            //      2. Enforce that always an NFT's immutable data must be unique in the resource's scope
            let id = NftTokenId::random();
            let mut immutable_data = Metadata::new();
            immutable_data.insert("emojis", emojis);
            let nft = NftToken::new(immutable_data, Vec::new());
            // here we assume (2) that immutable data is unique, 
            // so the minting will fail if another nft with the same emojis was minted previously
            ResourceManager::get(self.resource_address).mint_non_fungible(nft);

            // Mint a new token with a random ID
            let id = NftTokenId::random();
            self.mint_specific(id);

            (emoji_id, change)
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.resource_address).total_supply()
        }
    }
}
