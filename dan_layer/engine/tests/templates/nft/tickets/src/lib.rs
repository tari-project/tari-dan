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

#[derive(Debug, Clone, Encode, Decode, Default)]
pub struct Ticket {
    pub is_redeemed: bool,
}

/// Example of a ticket sale for event attendances. Every ticket is tracked individually.
/// It should also be possible to add custom metadata to each ticket (e.g. seat number) with optional uniqueness checks
/// The same logic can be used for supply chain tracking of products
#[template]
mod tickets {
    use super::*;

    pub struct TicketSeller {
        resource_address: ResourceAddress,
        tickets: Vault,
        price: Amount,
        earnings: Vault,
    }

    impl TicketSeller {
        // TODO: in this example we need to specify the payment resource, but there should be native support for Thaums
        pub fn new(payment_resource_address: ResourceAddress, initial_supply: usize, price: Amount, event_description: String) -> Self {
            // Create the non-fungible resource
            // TODO: restrict minting to only the owner
            let resource_address = ResourceBuilder::non_fungible()
                // The event description is common for all tickets
                .add_metadata("event", event_description)
                .build();

            // Mint the initial tickets
            let ticket_bucket = ResourceManager::get(resource_address)
                .mint_many_non_fungible(&Metadata::new(), &Ticket::default(), initial_supply);
            let tickets = Vault::from_bucket(ticket_bucket);
 
            let earnings = Vault::new_empty(payment_resource_address);

            Self {
                resource_address,
                tickets,
                price,
                earnings,
            }
        }

        // TODO: this method should only be allowed for the owner, when they want to increase attendance of the event
        pub fn mint_more_tickets(&mut self, supply: usize) {
            let ticket_bucket = ResourceManager::get(self.resource_address)
                .mint_many_non_fungible(&Metadata::new(), &Ticket::default(), supply);
            self.tickets.deposit(ticket_bucket);
        }

        // This method should be accesible to everyone
        // TODO: how do we ensure that the payment is in Thaums? On vault creation we specify the type of token?
        // TODO: return change (or check bucket.value() == required_amount)
        pub fn buy_ticket(&mut self, payment: Bucket) -> Bucket {
            self.earnings.deposit(payment);

            // no need to manually check that the tickes are all sold out, as the withdraw operation will fail automatically
            let ticket_bucket = self.tickets.withdraw(Amount(1));

            ticket_bucket
        }

        // TODO: badge system should allow only the component owner to reddem, or emit "redeemer" badges
        // TODO: pass the token id or use buckets? we need a way to ensure that the caller has the nft
        pub fn redeem_ticket(&self, id: NonFungibleId) {
            let resource_manager = ResourceManager::get(self.resource_address);
            let mut nft = resource_manager.get_non_fungible(&id);
            let mut data = nft.get_mutable_data::<Ticket>();
            data.is_redeemed = true;
            nft.set_mutable_data(&data);
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.resource_address).total_supply()
        }
    }
}
