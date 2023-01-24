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

/// Simple English-like auctions
/// The winner needs to claim the nft after the bidding period finishes. For simplicity, no marketplace fees are considered.
/// There exist a lot more approaches to auctions, we can highlight:
///     - Price descending, dutch-like auctions. The first bidder gets the nft right away, no need to wait or claim afterwards
///     - Blind auctions, were bids are not known until the end. This requires cryptography support, and implies that all bidder's funds will be locked until the end of the auction
pub struct Auction {
    // The NFT will be locked, so the user gives away control to the marketplace
    // There are other approaches to this, like just allowing the seller to complete and confirm the bid at the end
    // We could also allow auction cancel by the seller by returning a badge
    vault: Vault,

    // address of the account component of the seller
    seller_address: ComponentAddress,

    // minimum required price for a bid
    min_price: Option<Amount>,

    // price at which the NFT will be sold automatically
    buy_price: Option<Amount>,

    // Holds the current highest bidder, it's replaced when a new highest bidder appears
    highest_bid: Option<Bid>,

    // Time sensitive logic is a big issue, we need custom support for it. I see two options:
    //      1. Ad hoc protocol in the second layer to agree on time blocks (inside of a commitee? globally?) 
    //      2. Leverage the base layer block number
    // We are going with (2) here. But either way this means custom utils and that some external state influences execution
    ending_block: u64,
}

pub struct Bid {
    address: ComponentAddress,
    bid: Vault
}

// When a user just wants to sell the NFT at a fixed price
pub struct SellOrder {
    vault: Vault,
    seller_address: ComponentAddress,
    buy_price: Amount,
    // optional expiration of the sell order
    ending_block: Option<u64>,
}

#[template]
mod marketplace {
    use super::*;

    pub struct NftMarketplace {
        // TODO: can we ensure with the type system that we only want a single NFT address?
        auctions: HashMap<ResourceAddress, Auction>,
        sell_orders: HashMap<ResourceAddress, SellOrder>,
        sell_orders_badge_resource: ResourceAddress,
    }

    impl NftMarketplace {
        pub fn new() -> Self {
            Self {
                auctions: Hashmap::new(),
                sell_orders: Hashmap::new(),
                sell_orders_badge_resource: ResourceBuilder::non_fungible().build(),
            }
        }

        // TODO: how to ensure the bucket contains a single NFT item?
        pub fn start_auction(&mut self, nft_bucket: Bucket, min_price: Option<Amount>, buy_price: Option<Amount>, block_period: u64) {
            let auction = Auction {
                vault: Vault::from_bucket(nft_bucket),
                min_price,
                buy_price,
                highest_bid: None,
                // TODO: BaseLayerManager still does not exist in our current implementation
                ending_block: BaseLayerManager::current_block_heigth() + block_period,
            };
            self.auctions.insert(auction.vault.resource_address(), auction);
        }

        // returns a badge used to cancel the sell order in the future
        // the badge will contain a immutable metadata with a reference to the nft being sold
        pub fn place_sell_order(&mut self, nft_bucket: Bucket, buy_price: Amount, block_period: u64) -> Bucket {
            let sell_order = SellOrder {
                vault: Vault::from_bucket(nft_bucket),
                buy_price,
                ending_block: BaseLayerManager::current_block_heigth() + block_period,
            };
            let nft_address = sell_order.vault.resource_address();
            self.sell_orders.insert(nft_address, sell_order);

            // mint and return a badge to be used later for canceling the sell order
            let badge_id = NftTokenId::random();
            // the data MUST be immutable, to avoid security exploits (changing the nft which it points to afterwards)
            let mut badge_data = Metadata::new();
            badge_data
                .insert("nft_address", nft_address);
            ResourceManager::get(self.sell_orders_badge_resource).mint_non_fungible(badge_id, badge_data)
        }

        // pass the badge returned by "place_sell_order"
        // get the NFT back
        pub fn cancel_sell_order(&mut self, seller_badge: Bucket) -> Bucket {
            assert!(
                seller_badge.resource_address() == self.sell_orders_badge_resource,
                "Invalid seller badge resource"
            );

            // again, we assume that the metadata is immutable
            let nft_address = seller_badge.get_metadata("nft_address");
            
            let sell_order = self.sell_orders.get(nft_address)
                .expect("Sell order does not exist");
            let nft_bucket = sell_order.vault.withdraw_all();

            self.sell_orders.remove(nft_address);

            nft_bucket
        }

        // but a nft sell order directly
        // returns the nft and the payment change
        pub fn buy(&mut self, nft_address: ResourceAddress, payment: Bucket<Thaum>) -> (Bucket, Bucket<Thaum>) {
            let sell_order = self.sell_orders.get(nft_address)
                .expect("Sell order does not exist");

            assert!(
                BaseLayerManager::current_block_heigth() < sell_order.ending_block 
                "Sell order has expired"
            );

            // pay the seller
            let seller_account = AccountManager::get(sell_order.seller_address);
            // note that we do not neet to manually check that the payment is enough, the split will fail in that case
            let (price, change) = payment.split(sell_order.buy_price);
            // TODO: deposit_thaum does not exist yet, maybe we just want a way to get the resource address
            seller_account.deposit_thaum(price);

            // return the nft and change back to the buyer
            let nft_bucket = sell_order.vault.withdraw_all();
            sell_orders.remove(nft_address);
            (nft_bucket, change)
        }

        // returns the payment change in case that the bid meets the buying price
        pub fn bid(&mut self, bidder_account_address: ComponentAddress, nft_address: ResourceAddress, payment: Bucket<Thaum>) -> Option<(Bucket, Bucket<Thaum>)> {
            let mut auction = self.auctions.get_mut(nft_address)
                .expect("Auction does not exist");
            
            assert!(
                BaseLayerManager::current_block_heigth() < auction.ending_block 
                "Auction has expired"
            );

            // check that the minimum price (if set) is met
            if let Some(min_price) = auction.min_price {
                assert!(
                    payment.amount() >= min_price
                    "Minium price not met"
                );
            }

            // check if there is a previous bid placed
            if let Some(highest_bid) = auction.highest_bid {
                assert!(
                    payment.amount() > highest_bid.vault.amount()
                    "There is a higher bid placed"
                );

                // we need to pay back the previous highest bidder
                let previous_bidder_account = AccountManager::get(highest_bid.address);
                previous_bidder_account.deposit(highest_bid.vault.withdraw_all());
            }

            // if the bid meets the buying price, we process the sell immediatly
            if let Some(buy_price) = auction.buy_price {
                if payment.amount() >= buy_price {
                    // pay the seller
                    let (price, change) = payment.split(buy_price);
                    let seller_account = AccountManager::get(auction.seller_address);
                    seller_account.deposit_thaum(price);

                    // return the nft and change to the bidder
                    let nft_bucket = auction.vault.withdraw_all();
                    self.auctions.remove(nft_address);

                    return Some(nft_bucket, change);
                }
            }

            // set the new highest bid
            let new_higest_bid = Bid {
                address: bidder_account_address,
                bid: Vault::from_bucket(payment),
            };
            auction.highest_bid = Some(new_higest_bid);

            // at this point, this is a normal bid, so the bidder must wait for the auction to finish
            None
        }

        // used by a bid winner to get the nft they bid for
        // TODO: use badges or auth system to ensure that the caller is the account being passed as parameter
        pub fn claim_auction_nft(&mut self, bidder_account_address: ComponentAddress, nft_address: ResourceAddress) -> Bucket {
            let mut auction = self.auctions.get_mut(nft_address)
                .expect("Auction does not exist");
            
            assert!(
                BaseLayerManager::current_block_heigth() > auction.ending_block 
                "Auction is still in progress"
            );

            if let Some(highest_bid) = auction.highest_bid {
                // TODO: maybe issue badges instead of manual checking
                assert!(
                    highest_bid.address == bidder_account_address
                    "The caller is not the winner"
                );

                // pay the seller immediatly
                let seller_account = AccountManager::get(auction.seller_address);
                seller_account.deposit_thaum(auction.highest_bid.withdraw_all());

                // return the nft to the bidder
                let nft_bucket = auction.vault.withdraw_all();
                self.auctions.remove(nft_address);

                return nft_bucket;
            } else {
                panic!("The auction has ended with no winner")
            }
        }

        // used by a bid seller to receive the bid payment
        // TODO: use badges or auth system to ensure that the caller is the seller account passed as parameter
        pub fn claim_auction_payment(&mut self, seller_account_address: ComponentAddress, nft_address: ResourceAddress) -> Bucket<Thaum> {
            let mut auction = self.auctions.get_mut(nft_address)
                .expect("Auction does not exist");

            assert!(
                BaseLayerManager::current_block_heigth() > auction.ending_block 
                "Auction is still in progress"
            );

            // TODO: maybe issue badges instead of manual checking
            assert!(
                auction.seller_address == seller_account_address
                "The caller is not the seller"
            );

            if let Some(highest_bid) = auction.highest_bid {
                // return the nft to the bidder
                let bidder_account = AccountManager::get(highest_bid.address);
                let nft_bucket = auction.vault.withdraw_all();
                bidder_account.deposit(nft_bucket);

                // get the bid funds
                let seller_account = AccountManager::get(auction.seller_address);
                let payment = auction.highest_bid.withdraw_all();               
                self.auctions.remove(nft_address);

                return payment;
            } else {
                // The seller would need to manually call a "cancel_auction" method? not very ergonomic
                panic!("The auction has ended with no winner")
            }
        }

        // TODO: cancel auction? similar to "cancel_sell_order" to get the nft back to the seller

        // convenience methods for external APIs and interfaces
        // Support for advanced filtering (price ranges, auctions about to end, etc.) could be desirable
        pub fn get_auctions(&self) -> HashMap<NonFungibleTokenId, Auction> {
            self.auctions.clone()
        }
        pub fn get_sell_orders(&self) -> HashMap<NonFungibleTokenId, Auction> {
            self.auctions.clone()
        }
    }
}
