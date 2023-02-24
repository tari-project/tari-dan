//   Copyright 2023. The Tari Project
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
pub struct Sparkle {
    pub brightness: u32,
}

#[template]
mod sparkle_nft_template {
    use super::*;

    pub struct SparkleNft {
        resource_address: ResourceAddress,
        minted_nft_list: AddressList,
    }

    impl SparkleNft {
        pub fn new() -> Self {
            let resource_address = ResourceBuilder::non_fungible().build();
            let minted_nft_list = AddressList::new();

            Self {
                resource_address,
                minted_nft_list,
            }
        }

        pub fn mint(&mut self) -> Bucket {
            // Mint a new token with a random ID
            let id = NonFungibleId::random();
            self.mint_specific(id)
        }

        pub fn mint_specific(&mut self, id: NonFungibleId) -> Bucket {
            debug(format!("Minting {}", id));
            // These are characteristic of the NFT and are immutable
            let mut immutable_data = Metadata::new();
            immutable_data
                .insert("name", format!("Sparkle{}", id))
                .insert("image_url", format!("https://nft.storage/sparkle{}.png", id));

            // Mint the NFT, this will fail if the token ID already exists
            let mut res_manager = ResourceManager::get(self.resource_address);
            let nft_bucket = res_manager.mint_non_fungible(id.clone(), &immutable_data, &Sparkle { brightness: 0 });

            // Add a NFT reference in the list, to keep track of it
            let index = res_manager.total_supply().value() as u64;
            let nft_address: Address = NonFungibleAddress::new(self.resource_address, id).into();
            // TODO: the DAN layer should resolve the index of the list transparently to he user
            self.minted_nft_list.push(index + 1, nft_address);
            
            // return a bucket with the newly minted nft
            nft_bucket
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.resource_address).total_supply()
        }

        pub fn inc_brightness(&mut self, id: NonFungibleId, brightness: u32) {
            debug(format!("Increase brightness on {} by {}", id, brightness));
            self.with_sparkle_mut(id, |data| {
                data.brightness = data.brightness.checked_add(brightness).expect("Brightness overflow");
            });
        }

        fn with_sparkle_mut<F: FnOnce(&mut Sparkle)>(&self, id: NonFungibleId, f: F) {
            let resource_manager = ResourceManager::get(self.resource_address);
            let mut nft = resource_manager.get_non_fungible(&id);
            let mut data = nft.get_mutable_data::<Sparkle>();
            f(&mut data);
            nft.set_mutable_data(&data);
        }

        pub fn burn(&mut self, mut bucket: Bucket) {
            // this check is actually not needed, but with it we cover the "bucket.resource_type" method
            assert!(
                bucket.resource_type() == ResourceType::NonFungible,
                "The resource is not a NFT"
            );
            assert!(
                bucket.resource_address() == self.resource_address,
                "Cannot burn bucket not from this collection"
            );
            debug(format!("Burning bucket {} containing {}", bucket.id(), bucket.amount()));
            // This is all that's required, typically the template would not need to include a burn function because a
            // native instruction can be used instead
            bucket.burn();
        }
    }
}
