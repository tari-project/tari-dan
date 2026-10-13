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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pool {
    A,
    B,
}

impl Pool {
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

#[template]
mod template {
    use tari_template_lib::resource::TOKEN_SYMBOL;

    use super::*;

    pub struct TwoResourceLiquidityPool {
        vault_a: Vault,
        vault_b: Vault,
        lp_resource: ResourceManager,
    }

    impl TwoResourceLiquidityPool {
        // Creates a new two-resource liquidity pool component for the resources A and B
        pub fn create(
            owner_rule: OwnerRule,
            contribute_and_redeem_rule: AccessRule,
            a_addr: ResourceAddress,
            b_addr: ResourceAddress,
            mut metadata: Metadata,
            address_allocation: Option<ComponentAddressAllocation>,
        ) -> Component<Self> {
            // check that the resource pair is correct
            assert_ne!(a_addr, b_addr, "The resources of the pair must be different");
            Self::check_resource_is_fungible(a_addr);
            Self::check_resource_is_fungible(b_addr);

            // Set token symbol to "LP" if not provided
            if !metadata.contains_key(TOKEN_SYMBOL) {
                metadata.insert(TOKEN_SYMBOL, "LP");
            }
            let address_allocation =
                address_allocation.unwrap_or_else(|| CallerContext::allocate_component_address(None));
            // LP supply is a claim on the reserves, so only the pool, which mints against a contribution and burns
            // against a redemption, may change it.
            let pool_only = rule!(component(address_allocation.get_address()));

            // create the lp resource
            let lp_resource = ResourceBuilder::public_fungible()
                .with_divisibility(0)
                .with_access_rules(
                    ResourceAccessRules::new()
                        .mintable(pool_only.clone(), LOCKED)
                        .burnable(pool_only, LOCKED),
                )
                .with_owner_rule(owner_rule.clone())
                .with_metadata(metadata)
                .build();

            let vault_a = Vault::new_empty(a_addr);
            let vault_b = Vault::new_empty(b_addr);

            Component::new(Self {
                vault_a,
                vault_b,
                lp_resource: ResourceManager::get(lp_resource),
            })
            .with_owner_rule(owner_rule)
            .with_address_allocation(address_allocation)
            .with_access_rules(
                ComponentAccessRules::new()
                    // Only owners can rebalance the pool by adding/removing liquidity directly
                    .add_method_rule("protected_add_liquidity", AccessRule::DenyAll)
                    .add_method_rule("protected_remove_liquidity", AccessRule::DenyAll)
                    .add_method_rule("contribute", contribute_and_redeem_rule.clone())
                    .add_method_rule("redeem", contribute_and_redeem_rule)
                    .default(AccessRule::AllowAll),
            )
            .create()
        }

        /// Contributes liquidity to the pool, minting LP tokens in return.
        ///
        /// To preserve the reserve ratio the pool only takes the matching portion of one of the input buckets, so the
        /// remainder of the other is returned to the caller as change. Returns `(lp_tokens, change_a, change_b)` where
        /// `change_a`/`change_b` correspond to `bucket_a`/`bucket_b` and may be empty.
        pub fn contribute(&mut self, mut bucket_a: Bucket, mut bucket_b: Bucket) -> (Bucket, Bucket, Bucket) {
            // Potentially saves binary space
            const OVERFLOW_MSG: &str = "Overflow when calculating LP token mint amount";

            // check that the buckets are correct
            let resource_a = bucket_a.resource_address();
            let resource_b = bucket_b.resource_address();
            assert_ne!(resource_a, resource_b, "The resources must be different");

            // contribution amounts
            let a_amount = bucket_a.amount().into_precision_amount();
            let b_amount = bucket_b.amount().into_precision_amount();

            if a_amount.is_zero() || b_amount.is_zero() {
                panic!("Cannot contribute zero amount of liquidity");
            }

            let pool_a = self.get_pool_from_resource(resource_a);
            let pool_b = self.get_pool_from_resource(resource_b);

            // Get the reserves for each pool
            let reserve_a = self.get_pool_vault(pool_a).balance().into_precision_amount();
            let reserve_b = self.get_pool_vault(pool_b).balance().into_precision_amount();

            let lp_supply = self.lp_total_supply().into_precision_amount();

            let (lp_mint_amount, a_contribution, b_contribution) = match (
                lp_supply.is_positive(),
                reserve_a.is_positive(),
                reserve_b.is_positive(),
            ) {
                // No LP tokens exist yet: this is the initial liquidity provision. With no LP outstanding there is
                // no existing price to preserve, so the first provider sets it: take the full contribution and mint
                // LP equal to the geometric mean `sqrt(a * b)`.
                (false, false, false) => {
                    // TODO: possible loss of precision even with 192 bit integer
                    let mint = a_amount
                        .checked_mul(b_amount)
                        .and_then(|product| product.checked_sqrt())
                        .expect(OVERFLOW_MSG);

                    (mint, a_amount, b_amount)
                },
                (false, _, _) => {
                    // The reserves belong to the LP holders, and `protected_add_liquidity` refuses to fund a pool
                    // with no LP outstanding, so this is an inconsistent state.
                    panic!("Inconsistent pool state: zero LP supply with non-zero reserve");
                },
                (true, true, true) => {
                    // Normal case: existing LP supply and non-zero reserves
                    // calculate the amount each contribution can be contributed to keep the ratio of the reserves the
                    // same

                    // Multiply before dividing: `PrecisionAmount` is a 192-bit *integer*, so dividing first
                    // truncates the ratio to zero whenever a contribution is smaller than its reserve. The wide
                    // intermediate holds the product without overflow.
                    let required_contribution_a = a_amount
                        .checked_mul(reserve_b)
                        .and_then(|num| num.checked_div(reserve_a))
                        .map(|b_required| (a_amount, b_required))
                        .expect(OVERFLOW_MSG);

                    let required_contribution_b = b_amount
                        .checked_mul(reserve_a)
                        .and_then(|num| num.checked_div(reserve_b))
                        .map(|a_required| (a_required, b_amount))
                        .expect(OVERFLOW_MSG);

                    [required_contribution_a, required_contribution_b]
                        .into_iter()
                        // filter only the contributions that are less than or equal to what the user provided
                        .filter(|(c_a, c_b)| *c_a <= a_amount && *c_b <= b_amount)
                        .map(|(c_a, c_b)| {
                            let mint = c_a
                                .checked_mul(lp_supply)
                                .and_then(|num| num.checked_div(reserve_a))
                                .expect(OVERFLOW_MSG);
                            (mint, c_a, c_b)
                        })
                        .max_by(|(mint_a, _, _), (mint_b, _, _)| mint_a.cmp(mint_b))
                        .expect("Insufficient contribution amounts for existing pool reserves")
                },
                (true, _, _) => {
                    // One or both reserves are zero with existing LP supply. LP Supply should always represent
                    // reserves, so this is an inconsistent state.
                    panic!("Inconsistent pool state: non-zero LP supply with zero reserve");
                },
            };

            // convert back to normal Amount
            let lp_mint_amount = Amount::try_from(lp_mint_amount).expect("LP mint amount conversion failed");
            let a_contribution = Amount::try_from(a_contribution).expect("A contribution conversion failed");
            let b_contribution = Amount::try_from(b_contribution).expect("B contribution conversion failed");

            // Reject dust contributions that round down to a zero contribution or zero mint, rather than failing
            // later with an opaque `Bucket::take` assertion.
            assert!(
                a_contribution.is_positive() && b_contribution.is_positive() && lp_mint_amount.is_positive(),
                "Contribution too small relative to pool reserves to mint any LP tokens"
            );

            // mint and return the new lp tokens
            let mint_lp_tokens = self.lp_resource.mint_fungible(lp_mint_amount);

            let contributed_a = bucket_a.take(a_contribution);
            let contributed_b = bucket_b.take(b_contribution);

            // deposit the liquidity to the pool
            self.vault_a.deposit(contributed_a);
            self.vault_b.deposit(contributed_b);

            emit_event("contribute", metadata![
                "resource_a" => resource_a,
                "resource_b" => resource_b,
                "minted_lp_tokens" => lp_mint_amount,
                "contributed_a" => a_contribution,
                "contributed_b" => b_contribution,
            ]);

            // Return the LP tokens plus the unused remainder of each input bucket as change. `bucket_a`/`bucket_b`
            // now hold only what was not contributed; one is usually empty, which is harmless to return.
            (mint_lp_tokens, bucket_a, bucket_b)
        }

        pub fn redeem(&mut self, lp_bucket: Bucket) -> (Bucket, Bucket) {
            // check that the bucket is correct
            let lp_resource = self.lp_resource.resource_address();
            let bucket_resource = lp_bucket.resource_address();
            if bucket_resource != lp_resource {
                panic!(
                    "The provided bucket {} must be pool LP resource {}",
                    bucket_resource, lp_resource
                );
            }

            let redeem_amount = lp_bucket.amount();
            let lp_total_supply = self.lp_total_supply();

            let (a_amount, b_amount) = self.calculate_redemption_amounts(lp_total_supply, redeem_amount);

            // A redemption so small it rounds down to zero of either reserve would otherwise abort with an opaque
            // withdraw error (or, worse, return nothing). Reject it up front before burning the LP tokens.
            assert!(
                a_amount.is_positive() && b_amount.is_positive(),
                "Redemption amount too small to withdraw any tokens from the pool"
            );

            // withdraw the redeemed amounts from the pool
            let a_bucket = self.vault_a.withdraw(a_amount);
            let b_bucket = self.vault_b.withdraw(b_amount);

            // burn the redeemed lp tokens
            lp_bucket.burn();

            emit_event("redeem", metadata![
                "lp_redeemed" => redeem_amount,
                "resource_a" => a_bucket.resource_address(),
                "amount_a" => a_amount,
                "resource_b" => b_bucket.resource_address(),
                "amount_b" => b_amount,
            ]);

            (a_bucket, b_bucket)
        }

        pub fn protected_add_liquidity(&mut self, bucket: Bucket) {
            // Reserves added here mint no LP, so they accrue to the existing LP holders. A pool with no LP
            // outstanding has no holders to credit and is funded only through `contribute`.
            assert!(
                self.lp_total_supply().is_positive(),
                "Cannot add liquidity to a pool with no LP supply: bootstrap it with `contribute`"
            );

            // check that the buckets are correct
            let resource = bucket.resource_address();
            emit_event("add_liquidity", metadata![
                "resource_address" => resource,
                "amount" => bucket.amount(),
            ]);

            // add the liquidity to the pool
            let pool = self.get_pool_from_resource(resource);
            self.deposit_revealed(pool, bucket);
        }

        pub fn protected_remove_liquidity(&mut self, resource_address: ResourceAddress, amount: Amount) -> Bucket {
            let pool = self.get_pool_from_resource(resource_address);
            emit_event("remove_liquidity", metadata![
                "resource_address" => resource_address,
                "amount" => amount,
            ]);

            self.get_pool_vault(pool).withdraw(amount)
        }

        /// A simple swap method that uses a constant product formula without fees.
        /// This method makes the liquidity pool conform to the swap interface.
        /// A confidential input must hold only revealed funds.
        pub fn swap(&mut self, input_bucket: Bucket) -> Bucket {
            self.swap_constant_product(input_bucket)
        }

        /// A simple constant product swap implementation without fees. This is provided as a convenience but
        /// may not be generally useful.
        /// Users of this template can implement their own swap logic with fees as needed and use an instance of this
        /// pool template as a sub-component.
        pub fn swap_constant_product(&mut self, input_bucket: Bucket) -> Bucket {
            let input_resource = input_bucket.resource_address();

            let input_amount = input_bucket.amount();
            if input_amount.is_zero() {
                panic!("Cannot swap zero amount");
            }

            let input_pool = self.get_pool_from_resource(input_resource);
            let output_pool = input_pool.other();

            let input_reserve = self.get_pool_vault(input_pool).balance();
            let output_reserve = self.get_pool_vault(output_pool).balance();

            // The price is the ratio of the reserves, which is undefined while either is empty.
            assert!(
                input_reserve.is_positive() && output_reserve.is_positive(),
                "Pool has no liquidity for one of its resources"
            );

            // Simple constant product formula without fees
            // Δy = y.Δx / (X + Δx)
            // Computed in 192-bit precision to avoid overflow in the `y·Δx` product for large reserves.
            let dx = input_amount.into_precision_amount();
            let output_amount = dx
                .checked_mul(output_reserve.into_precision_amount())
                .and_then(|num| {
                    input_reserve
                        .into_precision_amount()
                        .checked_add(dx)
                        .and_then(|denom| num.checked_div(denom))
                })
                .expect("Overflow in swap calculation");
            let output_amount = Amount::try_from(output_amount).expect("swap output conversion failed");

            // A swap input too small to move any of the output reserve would otherwise abort with an opaque
            // withdraw error. Reject it with a clear message.
            assert!(output_amount.is_positive(), "Swap input too small to yield any output");

            // Perform the swap
            self.deposit_revealed(input_pool, input_bucket);
            let output_bucket = self.get_pool_vault(output_pool).withdraw(output_amount);

            emit_event("swap", metadata![
                "input_resource" => input_resource,
                "input_amount" => input_amount,
                "output_resource" => output_bucket.resource_address(),
                "output_amount" => output_amount,
            ]);

            output_bucket
        }

        pub fn get_redemption_value(&self, lp_redeem_amount: Amount) -> (Amount, Amount) {
            let lp_total_supply = self.lp_total_supply();

            if !lp_redeem_amount.is_positive() || lp_redeem_amount > lp_total_supply {
                panic!(
                    "Invalid redemption amount {} (total supply {})",
                    lp_redeem_amount, lp_total_supply
                );
            }

            self.calculate_redemption_amounts(lp_total_supply, lp_redeem_amount)
        }

        fn calculate_redemption_amounts(&self, lp_total_supply: Amount, lp_redeem_amount: Amount) -> (Amount, Amount) {
            assert!(
                lp_total_supply.is_positive(),
                "Cannot redeem from a pool with no LP supply"
            );

            // get the pool reserve. Compute in 192-bit precision and multiply before dividing: dividing the LP ratio
            // first truncates it to zero for any partial redemption (since these are integers), and the wide
            // intermediate avoids overflow in `lp_redeem_amount * reserve`.
            let a_reserve = self.vault_a.balance().into_precision_amount();
            let b_reserve = self.vault_b.balance().into_precision_amount();
            let lp_total_supply = lp_total_supply.into_precision_amount();
            let lp_redeem_amount = lp_redeem_amount.into_precision_amount();

            // amount_owed = lp_redeem_amount * reserve / lp_total_supply (rounded down in favour of the pool)
            let a_amount_owed = lp_redeem_amount
                .checked_mul(a_reserve)
                .and_then(|num| num.checked_div(lp_total_supply))
                .expect("Amount overflow when calculating redemption value for resource A");

            let b_amount_owed = lp_redeem_amount
                .checked_mul(b_reserve)
                .and_then(|num| num.checked_div(lp_total_supply))
                .expect("Amount overflow when calculating redemption value for resource B");

            (
                Amount::try_from(a_amount_owed).expect("redemption amount A conversion failed"),
                Amount::try_from(b_amount_owed).expect("redemption amount B conversion failed"),
            )
        }

        pub fn get_pool_balances(&self) -> (Amount, Amount) {
            (self.vault_a.balance(), self.vault_b.balance())
        }

        fn get_pool_from_resource(&self, resource_address: ResourceAddress) -> Pool {
            if self.vault_a.resource_address() == resource_address {
                Pool::A
            } else if self.vault_b.resource_address() == resource_address {
                Pool::B
            } else {
                panic!("Resource {} is not in the pool", resource_address);
            }
        }

        fn get_pool_vault(&self, pool: Pool) -> &Vault {
            match pool {
                Pool::A => &self.vault_a,
                Pool::B => &self.vault_b,
            }
        }

        /// Deposits a caller's bucket into the pool. The pool prices and pays out revealed funds only, so a
        /// confidential bucket must carry no commitments: none of the pool's methods could ever withdraw them.
        fn deposit_revealed(&self, pool: Pool, bucket: Bucket) {
            if bucket.resource_type() == ResourceType::Confidential {
                bucket.assert_contains_no_confidential_funds();
            }
            self.get_pool_vault(pool).deposit(bucket);
        }

        fn lp_total_supply(&self) -> Amount {
            self.lp_resource.total_supply()
        }

        fn check_resource_is_fungible(resource: ResourceAddress) {
            let resource_type = ResourceManager::get(resource).resource_type();
            assert!(
                matches!(
                    resource_type,
                    ResourceType::Fungible | ResourceType::Confidential | ResourceType::Stealth
                ),
                "Resource {} is not fungible (Fungible, Stealth, Confidential)",
                resource
            );
        }
    }
}
