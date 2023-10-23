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
    pub struct TariSwapPool {
        pools: HashMap<ResourceAddress, Vault>,
        lp_resource: ResourceAddress,
        fee: u16,
    }

    impl TariSwapPool {

        // Initialises a new pool component for for the pool A - B
        // the fees is represented as a per-mil quantity (e.g. "1" represents "0.1%")
        pub fn new(a_addr: ResourceAddress, b_addr: ResourceAddress, fee: u16) -> Self {
            // check that the the resource pair is correct
            assert!(a_addr != b_addr, "The resources of the pair must be different");
            Self::check_resource_is_fungible(a_addr);
            Self::check_resource_is_fungible(b_addr);

            // the fee represents a percentage, so it must be between 0 and 100
            let valid_fee_range = 0..1000;
            assert!(valid_fee_range.contains(&fee), "Invalid fee {}", fee);

            // create the vaults to store the funds
            let mut pools = HashMap::new();
            pools.insert(a_addr, Vault::new_empty(a_addr));
            pools.insert(b_addr, Vault::new_empty(b_addr));

            // create the lp resource
            // TODO: add lp resource minting/burning security, only this component should be allowed
            let lp_resource = ResourceBuilder::fungible().with_token_symbol("LP").build();

            Self {
                pools,
                lp_resource,
                fee,
            }
        }

        // swap A tokens for B tokens or viceversa
        pub fn swap(&mut self, input_bucket: Bucket, output_resource: ResourceAddress) -> Bucket {
            // check that the parameters are correct
            let input_resource = input_bucket.resource_address();
            self.check_pool_resources(input_resource, output_resource);

            // get the data needed to calculate the pool rebalancing
            let input_pool_balance = self.get_pool_balance(input_resource);
            let output_pool_balance = self.get_pool_balance(output_resource);

            // check that the pools are not empty, to prevent division by 0 errors later
            assert!(!input_pool_balance.is_zero(), "The pool for resource '{}' is empty", input_resource);
            assert!(!output_pool_balance.is_zero(), "The pool for resource '{}' is empty", output_resource);

            // apply the fee to the input bucket
            // so the user will get a lesser amout of tokens than the theoritical (for the gain of the LP holders)
            let input_bucket_balance = input_bucket.amount().value();
            let effective_input_balance = input_bucket_balance - (input_bucket_balance * (self.fee as i64)) / 1000;
            let effective_input_balance = Amount::new(effective_input_balance);

            // recalculate the new vault balances for the swap
            // constant product AMM formula is "k = a * b"
            // so the new output vault balance should be "b = k / a"
            let k = input_pool_balance * output_pool_balance;
            let new_input_pool_balance = input_pool_balance + effective_input_balance;
            let new_output_pool_balance = k / new_input_pool_balance;

            // calculate the amount of output tokens to return to the user
            let output_bucket_amount = output_pool_balance - new_output_pool_balance;
            
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

        pub fn remove_liquidity(&mut self, mut lp_bucket: Bucket) -> (Bucket, Bucket) {
            assert!(lp_bucket.resource_address() == self.lp_resource, "Invalid LP resource");

            // get the pool information
            let a_resource = self.get_a_resource();
            let a_balance = self.get_pool_balance(a_resource).value() as f64;
            let b_resource = self.get_b_resource();
            let b_balance = self.get_pool_balance(b_resource).value() as f64;

            // calculate the amount of tokens to take from each pool
            let lp_ratio = lp_bucket.amount().value() as f64 / self.lp_total_supply().value() as f64;
            let a_amount = Amount::new((lp_ratio * a_balance).ceil() as i64);
            let b_amount = Amount::new((lp_ratio * b_balance).ceil() as i64);

            // burn the LP tokens
            lp_bucket.burn();

            // return the pool tokens
            let a_bucket = self.pools.get_mut(&a_resource).unwrap().withdraw(a_amount);
            let b_bucket = self.pools.get_mut(&b_resource).unwrap().withdraw(b_amount);
            (a_bucket, b_bucket)
        }

        pub fn get_a_resource(&self) -> ResourceAddress {
            *self.pools.keys().nth(0).unwrap()
        }

        pub fn get_b_resource(&self) -> ResourceAddress {
            *self.pools.keys().nth(1).unwrap()
        }

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

        pub fn fee(&self) -> u16 {
            self.fee
        }

        fn check_pool_resources(&self, a_resource: ResourceAddress, b_resource: ResourceAddress) {
            assert!(a_resource != b_resource, "The resource addresses are the same");
            assert!(self.pools.contains_key(&a_resource), "The resource {} is not in the pool", a_resource);
            assert!(self.pools.contains_key(&b_resource), "The resource {} is not in the pool", b_resource);
        }

        fn check_resource_is_fungible(resource: ResourceAddress) {
            assert!(
                ResourceManager::get(resource).resource_type() == ResourceType::Fungible,
                "Resource {} is not fungible", resource
            );
        }
    }
}
