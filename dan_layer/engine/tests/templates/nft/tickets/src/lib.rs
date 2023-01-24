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

/// Example of a ticket sale for event attendances. Every ticket is tracked individually, but they do not hold any unique individual data.
/// It should also be possible to add custom metadata to each ticket (e.g. seat number) with optional uniqueness checks
/// The same logic can be used for supply chain tracking of products
#[template]
mod tickets {
    use super::*;

    pub struct TicketSeller {
        resource_address: ResourceAddress,
        // TODO: should we allow creating generic types for resources (Vault<Ticket>)? how are addresses resolved in that case?
        tickets: Vault,
        price: Amount,
        earnings: Vault<Thaum>,
    }

    impl TicketSeller {
        pub fn new(initial_supply: u64, price: Amount, event_metadate: Metadata) -> Self {
            let resource_address = ResourceBuilder::non_fungible()
                // The event metadata is common for all tickets, so it's stored only once in the resource, not in every ticket individually
                .with_metadata(event_metadata)
                .build();

            // Create the non-fungible resource with empty initial supply
            // TODO: restrict minting to only the owner
            let ticket_bucket = ResourceBuilder::get(resource_address)
                .add_supply(initial_supply)
                .build_bucket();

            // TODO: how do you initialize a Thaum vault? Could it be similar with non-Thaum fungible resources?    
            let earnings = Vault::new_empty::<Thaum>();

            Self {
                resource_address,
                tickets: Vault::from_bucket(ticket_bucket),
                price,
                earnings,
            }
        }

        // TODO: this method should only be allowed for the owner, when they want to increase attendance of the event
        pub fn mint_more_tickets(&mut self, supply: u64) {
            let ticket_bucket = ResourceBuilder::get(resource_address)
                .add_supply(supply)
                .build_bucket();
            self.tickets.deposit(ticket_bucket);
        }

        // This method should be accesible to everyone
        // TODO: how do we add generics to custom resource buckets? The resource addresses must be passed around somehow
        pub fn sell_ticket(&mut self, payment: Bucket<Thaum>) -> (Bucket, Bucket<Thaum>) {
            // no need to manually check the amount, as the split operation will fail if not enough funds
            let (cost, change) = payment.split(self.mint_price);
            self.earnings.put(cost);

            // no need to manually check that the tickes are all sold out, as the withdraw operation will fail automatically
            let ticket_bucket = self.tickets.withdraw(1);

            (ticket_bucket, change)
        }

        pub fn total_supply(&self) -> Amount {
            ResourceManager::get(self.address).total_supply()
        }
    }
}
