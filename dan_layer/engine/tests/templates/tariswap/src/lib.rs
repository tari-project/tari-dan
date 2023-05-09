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

use tari_template_abi::rust::collections::HashMap;
use tari_template_lib::prelude::*;

#[template]
mod tariswap {
    use super::*;

    // Constant product AMM
    // TODO: liquidity providing
    // TODO: pay market fee to LP holders (market fee as a constructor param)
    // TODO: lp resource minting/burning security
    pub struct TariSwapPool {
        pool_vaults: HashMap<ResourceAddress, Vault>,
    }

    impl TariSwapPool {

        // Initialises a new pool component for for the pool A - B
        pub fn new(a: Bucket, b: Bucket) -> Self {
            // check the the resource pair is correct
            assert!(a.resource_address() != b.resource_address(), "The tokens to swap are the same");
            assert!(
                a.resource_type() == ResourceType::Fungible,
                "Resource 'a' is not fungible"
            );
            assert!(
                b.resource_type() == ResourceType::Fungible,
                "Resource 'b' is not fungible"
            );

            // create the vaults to store the funds
            let mut pool_vaults = HashMap::new();
            pool_vaults.insert(a.resource_address(), Vault::new_empty(a.resource_address()));
            pool_vaults.insert(b.resource_address(), Vault::new_empty(b.resource_address()));

            Self {
                pool_vaults,
            }
        }

        // swap A tokens for B tokens or viceversa
        pub fn swap(&mut self, input_bucket: Bucket, output_resource: ResourceAddress) -> Bucket {
            // get the data needed to calculate the pool rebalancing
            let input_resource = input_bucket.resource_address();
            assert!(input_resource != output_resource, "The resource addresses are the same");
            let input_balance = self.get_pool_balance(input_resource);
            let output_balance = self.get_pool_balance(output_resource);

            // recalculate the new vault balances for the swap
            // constant product AMM formula is "k = a * b"
            // so the new output vault balance should be "b = k / a"
            let k = input_balance * output_balance;
            let new_input_balance = input_balance + input_bucket.amount();
            let new_output_balance = k / new_input_balance;

            // calculate the amount of output tokens to return to the user
            let output_bucket_amount = output_balance - new_output_balance;

            // perform the swap
            self.pool_vaults.get_mut(&input_resource).unwrap().deposit(input_bucket);
            self.pool_vaults.get_mut(&output_resource).unwrap().withdraw(output_bucket_amount)
        }

        // TODO: add liquidity

        // TODO: remove liquidity


        // public utility methods
        pub fn get_pool_balances(&self) -> HashMap<ResourceAddress, Amount> {
            let mut balances = HashMap::new();

            for (resource, vault) in &self.pool_vaults {
                balances.insert(resource.clone(), vault.balance());
            }

            balances
        }

        
        pub fn get_pool_balance(&self, resource_address: ResourceAddress) -> Amount {
            let vault = self.pool_vaults.get(&resource_address)
                .unwrap_or_else(|| panic!("Resource {} is not in the pool", resource_address));
            vault.balance()
        }
         

        // TODO: get LP token address and supply

        // TODO: get token price
    }
}
