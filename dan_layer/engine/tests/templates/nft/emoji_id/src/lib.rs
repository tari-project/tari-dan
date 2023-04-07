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

use std::{fmt, vec::Vec};

use tari_template_lib::prelude::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Hash)]
#[repr(i32)]
pub enum Emoji {
    Smile = 0x00,
    Sweat = 0x01,
    Laugh = 0x02,
    Wink = 0x03,
}

impl fmt::Display for Emoji {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Emoji::Smile => "\u{1F600}",
            Emoji::Sweat => "\u{1F605}",
            Emoji::Laugh => "\u{1F602}",
            Emoji::Wink => "\u{1F609}",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Hash)]
pub struct EmojiId(Vec<Emoji>);

impl fmt::Display for EmojiId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for emoji in &self.0 {
            write!(f, "{}", emoji)?;
        }
        Ok(())
    }
}

/// This template implements a classic, first come first served, constant price NFT drop
#[template]
mod emoji_id {
    use super::*;

    pub struct EmojiIdMinter {
        max_emoji_id_len: u64,
        mint_price: Amount,
        resource_address: ResourceAddress,
        earnings: Vault,
    }

    impl EmojiIdMinter {
        // TODO: in this example we need to specify the payment resource, but there should be native support for Thaums
        // TODO: decoding fails if "max_emoji_id_len" is usize instead of u64, we may need to add support for it
        pub fn new(payment_resource_address: ResourceAddress, max_emoji_id_len: u64, mint_price: Amount) -> Self {
            // Create the non-fungible resource with empty initial supply
            let resource_address = ResourceBuilder::non_fungible("emo").build();
            let earnings = Vault::new_empty(payment_resource_address);
            Self {
                max_emoji_id_len,
                mint_price,
                resource_address,
                earnings,
            }
        }

        // TODO: return change (or check bucket.value() == required_amount)
        pub fn mint(&mut self, emoji_id: EmojiId, payment: Bucket) -> Bucket {
            assert!(
                !emoji_id.0.is_empty() && emoji_id.0.len() as u64 <= self.max_emoji_id_len,
                "Invalid Emoji ID length"
            );

            // process the payment
            assert_eq!(payment.amount(), self.mint_price, "Invalid payment amount");
            // no need to manually check that the payment is in the same resource that we are accepting ...
            // ... the deposit will fail if it's different
            self.earnings.deposit(payment);

            // mint a new emoji id
            // TODO: how do we ensure uniqueness of emoji ids? Two options:
            //      1. Derive the nft id from the emojis
            //      2. Enforce that always an NFT's immutable data must be unique in the resource's scope
            //      3. Ad-hoc uniqueness fields in a NFT resource
            // We are going with (1) for now
            let id = NonFungibleId::from_string(emoji_id.to_string());
            let mut immutable_data = Metadata::new();
            immutable_data.insert("emoji id", emoji_id.to_string());

            // if a previous emoji id was minted with the same emojis, the hash will be the same
            // so consensus will fail when running "mint_non_fungible"
            let emoji_id_bucket =
                ResourceManager::get(self.resource_address).mint_non_fungible(id, &immutable_data, &{});

            emoji_id_bucket
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.resource_address).total_supply()
        }
    }
}
