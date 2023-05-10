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
        pools: HashMap<ResourceAddress, Vault>,
        lp_resource: ResourceAddress,
    }

    impl TariSwapPool {

        // Initialises a new pool component for for the pool A - B
        pub fn new(a_addr: ResourceAddress, b_addr: ResourceAddress) -> Self {
            // check the the resource pair is correct
            assert!(a_addr != b_addr, "The resources of the pair must be different");
            assert!(
                ResourceManager::get(a_addr).resource_type() == ResourceType::Fungible,
                "Resource 'a' is not fungible"
            );
            assert!(
                ResourceManager::get(a_addr).resource_type() == ResourceType::Fungible,
                "Resource 'b' is not fungible"
            );

            // create the vaults to store the funds
            let mut pools = HashMap::new();
            pools.insert(a_addr, Vault::new_empty(a_addr));
            pools.insert(b_addr, Vault::new_empty(b_addr));

            // create the lp resource
            let lp_resource = ResourceBuilder::fungible("LP").build();

            Self {
                pools,
                lp_resource,
            }
        }

        // swap A tokens for B tokens or viceversa
        pub fn swap(&mut self, input_bucket: Bucket, output_resource: ResourceAddress) -> Bucket {
            // check that the parameters are correct
            let input_resource = input_bucket.resource_address();
            self.check_pool_resources(input_resource, output_resource);

            // get the data needed to calculate the pool rebalancing
            let input_balance = self.get_pool_balance(input_resource);
            let output_balance = self.get_pool_balance(output_resource);

            // check that the pools are not empty, to prevent division by 0 errors later
            assert!(!input_balance.is_zero(), "The pool for resource '{}' is empty", input_resource);
            assert!(!output_balance.is_zero(), "The pool for resource '{}' is empty", output_resource);

            // recalculate the new vault balances for the swap
            // constant product AMM formula is "k = a * b"
            // so the new output vault balance should be "b = k / a"
            let k = input_balance * output_balance;
            let new_input_balance = input_balance + input_bucket.amount();
            let new_output_balance = k / new_input_balance;

            // calculate the amount of output tokens to return to the user
            let output_bucket_amount = output_balance - new_output_balance;

            // perform the swap
            self.pools.get_mut(&input_resource).unwrap().deposit(input_bucket);
            self.pools.get_mut(&output_resource).unwrap().withdraw(output_bucket_amount)
        }

        pub fn add_liquidity(&mut self, a_bucket: Bucket, b_bucket: Bucket) -> Bucket {
            // check that the buckets are correct
            let a_resource = a_bucket.resource_address();
            let b_resource = b_bucket.resource_address();
            self.check_pool_resources(a_resource, b_resource);

            // extract the bucket amounts for later
            let a_amount = a_bucket.amount();
            let b_amount = b_bucket.amount();

            // add the liquidity to the pool
            self.pools.get_mut(&a_resource).unwrap().deposit(a_bucket);
            self.pools.get_mut(&b_resource).unwrap().deposit(b_bucket);

            // get the bucket/pool ratios
            let a_ratio = self.get_pool_ratio(a_resource, a_amount);
            let b_ratio = self.get_pool_ratio(b_resource, b_amount);

            // the amount of new lp tokens are proportional to the bucket-pool ratios
            let new_lp_amount = a_ratio * a_amount + b_ratio * b_amount;

            // mint and return the new lp tokens
            ResourceManager::get(self.lp_resource).mint_fungible(new_lp_amount)
        }

        // TODO: remove liquidity.
        // Right now we cannot implement it as we need to process tuple variables in the workspace
        // pub fn remove_liquidity(&mut self, lp_bucket: Bucket) -> (Bucket, Bucket)

        // public utility methods
        pub fn get_pool_balances(&self) -> HashMap<ResourceAddress, Amount> {
            let mut balances = HashMap::new();

            for (resource, vault) in &self.pools {
                balances.insert(resource.clone(), vault.balance());
            }

            balances
        }
        
        pub fn get_pool_balance(&self, resource_address: ResourceAddress) -> Amount {
            let vault = self.pools.get(&resource_address)
                .unwrap_or_else(|| panic!("Resource {} is not in the pool", resource_address));
            vault.balance()
        }

        pub fn get_pool_ratio(&self, resource: ResourceAddress, amount: Amount) -> Amount {
            let balance = self.get_pool_balance(resource);

            if balance == 0 {
                Amount::new(1)
            } else {
                amount / balance
            }
        }
         
        pub fn lp_resource(&self) -> ResourceAddress {
            self.lp_resource
        }

        pub fn lp_total_supply(&self) -> Amount {
            ResourceManager::get(self.lp_resource).total_supply()
        }

        fn check_pool_resources(&self, a_resource: ResourceAddress, b_resource: ResourceAddress) {
            assert!(a_resource != b_resource, "The resource addresses are the same");
            assert!(self.pools.contains_key(&a_resource), "The resource {} is not in the pool", a_resource);
            assert!(self.pools.contains_key(&b_resource), "The resource {} is not in the pool", b_resource);
        }
    }
}
