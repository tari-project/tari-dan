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

// TODO: Immutable/mutable properties of the NFT defined by a struct
// #[derive(Debug, Clone, Encode, Decode)]
// struct Sparkle {
//     pub brightness: u32,
// }

#[template]
mod sparkle_nft_template {
    use super::*;

    pub struct SparkleNft {
        address: ResourceAddress,
    }

    impl SparkleNft {
        pub fn new() -> Self {
            // Create the non-fungible resource with 1 token (optional)
            let tokens = [(NftTokenId::random(), NftToken::new(Metadata::new(), Vec::new()))];
            let address = ResourceBuilder::non_fungible()
                .with_token_symbol("SPKL")
                .with_tokens(tokens)
                .build();
            Self { address }
        }

        pub fn mint(&mut self) -> Bucket {
            // Mint a new token with a random ID
            let id = NftTokenId::random();
            self.mint_specific(id)
        }

        pub fn mint_specific(&mut self, id: NftTokenId) -> Bucket {
            // These are characteristic of the NFT and are immutable
            let mut immutable_data = Metadata::new();
            immutable_data
                .insert("name", format!("Sparkle{}", id))
                .insert("image_url", format!("https://nft.storage/sparkle{}.png", id));
            // TODO: Custom data that is mutable by the token owner (probably a serialized struct)
            // Mint the NFT, this will fail if the token ID already exists
            ResourceManager::get(self.address).mint_non_fungible(id, NftToken::new(immutable_data, Vec::new()))
        }
    }
}
