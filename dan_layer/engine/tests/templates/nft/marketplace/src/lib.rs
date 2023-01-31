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
use tari_template_lib::models::ComponentAddress;

/// Simple English-like auctions
/// The winner needs to claim the nft after the bidding period finishes. For simplicity, no marketplace fees are considered.
/// There exist a lot more approaches to auctions, we can highlight:
///     - Price descending, dutch-like auctions. The first bidder gets the nft right away, no need to wait or claim afterwards
///     - Blind auctions, were bids are not known until the end. This requires cryptography support, and implies that all bidder's funds will be locked until the end of the auction
#[derive(Debug, Clone, Encode, Decode)]
pub struct Auction {
    // The NFT will be locked, so the user gives away control to the marketplace
    // There are other approaches to this, like just allowing the seller to complete and confirm the bid at the end
    vault: Vault,

    // address of the account component of the seller
    seller_address: ComponentAddress,

    // TODO: there should be an easy way to specify that all payments are in Thaums
    //       but we don't have it yet, so each seler specifies the token that he wants to be paid in
    payment_resource_address: ResourceAddress,

    // minimum required price for a bid
    min_price: Option<Amount>,

    // price at which the NFT will be sold automatically
    buy_price: Option<Amount>,

    // Holds the current highest bidder, it's replaced when a new highest bidder appears
    highest_bid: Option<Bid>,

    // Time sensitive logic is a big issue, we need custom support for it. I see two options:
    //      1. Ad hoc protocol in the second layer to agree on timestamps (inside of a commitee? globally?) 
    //      2. Leverage the base layer block number (~3 minute intervals)
    //      3. Use the current epoch (~30 min intervals)
    // We are going with (3) here. But either way this means custom utils and that some external state influences execution
    ending_epoch: u64,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct Bid {
    address: ComponentAddress,
    bid: Vault
}

#[template]
mod marketplace {
    use super::*;

    pub struct NftMarketplace {
        // TODO: can we ensure with the type system that we only want a NFT address?
        auctions: HashMap<ResourceAddress, Auction>,
        seller_badge_resource: ResourceAddress,
    }

    impl NftMarketplace {
        pub fn new() -> Self {
            Self {
                auctions: HashMap::new(),
                seller_badge_resource: ResourceBuilder::non_fungible().build(),
            }
        }

        // TODO: how to ensure the bucket contains a single NFT item? Passing both the address and id seems a bit too unconvenient
        // returns a badge used to cancel the sell order in the future
        //      the badge will contain a immutable metadata with a reference to the nft being sold
        pub fn start_auction(&mut self, nft_bucket: Bucket, seller_address: ComponentAddress, payment_resource_address: ResourceAddress, min_price: Option<Amount>, buy_price: Option<Amount>, epoch_period: u64) -> Bucket {
            assert!(
                epoch_period > 0,
                "Invalid auction period"
            );  

            let auction = Auction {
                vault: Vault::from_bucket(nft_bucket),
                seller_address,
                payment_resource_address,
                min_price,
                buy_price,
                highest_bid: None,
                // TODO: current epoch retrieval does not exist in our current implementation
                ending_epoch: System::current_epoch() + epoch_period,
            };
            self.auctions.insert(auction.vault.resource_address(), auction);

            // mint and return a badge to be used later for (optionally) canceling the auction by the seller
            let badge_id = NonFungibleId::random();
            // the data MUST be immutable, to avoid security exploits (changing the nft which it points to afterwards)
            let mut immutable_data = Metadata::new();
            immutable_data
                .insert("nft_address", nft_address);
            ResourceManager::get(self.sell_orders_badge_resource).mint_non_fungible(badge_id, immutable_data, &());
        }

        // process the bid:
        //  - ignoring it (throws panic) if lower than the current highest
        //  - setting it up as the new highest bid 
        //  - performs the payment to the seller + the nft to the buyer if the buy price was met
        // TODO: we need to handle the payment change (or just check that the payment is within a valid range)
        pub fn bid(&mut self, bidder_account_address: ComponentAddress, nft_address: ResourceAddress, payment: Bucket) {
            let mut auction = self.auctions.get_mut(nft_address)
                .expect("Auction does not exist");
            
            assert!(
                System::current_epoch() < auction.ending_epoch,
                "Auction has expired"
            );

            assert!(
                auction.payment_resource_address < payment.resource_address(),
                "Invalid payment resource"
            );

            // check that the minimum price (if set) is met
            if let Some(min_price) = auction.min_price {
                assert!(
                    payment.amount() >= min_price,
                    "Minium price not met"
                );
            }

            // check if there is a previous bid placed
            if let Some(highest_bid) = auction.highest_bid {
                assert!(
                    payment.amount() > highest_bid.vault.amount(),
                    "There is a higher bid placed"
                );

                // we need to pay back the previous highest bidder
                let previous_bidder_account = AccountManager::get(highest_bid.address);
                previous_bidder_account.deposit(highest_bid.vault.withdraw_all());
            }

            // set the new highest bid     
            let new_higest_bid = Bid {
                address: bidder_account_address,
                bid: Vault::from_bucket(payment),
            };
            auction.highest_bid = Some(new_higest_bid);

            // if the bid meets the buying price, we process the sell immediatly
            if let Some(buy_price) = auction.buy_price {
                assert!(
                    payment.amount() <= buy_price,
                    "Payment is too big"
                );
                if payment.amount() == buy_price {
                    self.process_auction_payments(nft_address, auction);
                }
            }
        }

        // finish the auction by sending the NFT and payment to the respective accounts
        // used by a bid seller to receive the bid payment, or the buyer to get the NFT, whatever happens first
        pub fn finish_auction(&mut self, nft_address: ResourceAddress) {
            let mut auction = self.auctions.get_mut(nft_address)
                .expect("Auction does not exist");

            assert!(
                System::current_epoch() > auction.ending_epoch,
                "Auction is still in progress"
            );

            self.process_auction_payments(nft_address, auction);
        }

        // this method MUST be private, to avoid auction cancellation by unauthorized third parties
        fn process_auction_payments(&mut self, nft_address: ResourceAddress, auction: Auction) {
            let seller_account = AccountManager::get(auction.seller_address);
            let nft_bucket = auction.vault.withdraw_all();

            if let Some(highest_bid) = auction.highest_bid {
                // deposit the nft to the bidder
                let bidder_account = AccountManager::get(highest_bid.address);
                bidder_account.deposit(nft_address, nft_bucket);
                
                // deposit the funds to the seller
                let payment = highest_bid.bid.withdraw_all();
                seller_account.deposit(auction.payment_resource_address, payment);
            } else {
                // no bidders in the auction, so just return the NFT to the seller
                seller_account.deposit(nft_address, nft_bucket);
            }

            // TODO: burn the seller badge to avoid it being used again
            
            // remove the auction
            self.auctions.remove(nft_address);
        }

        // the seller wants to cancel the auction
        pub fn cancel_auction(&mut self, seller_badge: Bucket) {
            // we check that the badge is correct
            assert!(
                seller_badge.resource_address() == self.seller_badge_resource,
                "Invalid seller badge resource"
            );
            // again, we assume that the metadata in the badge is immutable
            let nft_address = seller_badge.into_resource().non_fungible().get_metadata("nft_address");     
            let mut auction = self.auctions.get(nft_address)
                .expect("Auction does not exist");

            // we assume that an auction cannot be cancelled if it has ended
            assert!(
                System::current_epoch() < auction.ending_epoch,
                "Auction has ended"
            );

            // we are canceling the bid
            // so we need to pay back the highest bidded (if there's one)
            if let Some(highest_bid) = auction.highest_bid {
                let bidder_account = AccountManager::get(highest_bid.address);
                bidder_account.deposit(highest_bid.vault.withdraw_all());
                auction.highest_bid = None;
            }
            
            // at this point there is no bidder
            // so the payment process will just send the NFT back to the seller
            self.process_auction_payments(nft_address, auction);
        }

        // convenience method for external APIs and interfaces
        // Support for advanced filtering (price ranges, auctions about to end, etc.) could be desirable
        // Can this method be called without paying fees? We also want to ensure the results come from consensus
        pub fn get_auctions(&self) -> HashMap<ResourceAddress, Auction> {
            self.auctions.clone()
        }
    }
}
