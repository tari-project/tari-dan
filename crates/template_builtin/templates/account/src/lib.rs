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

type VaultMap = PrehashedMap<ResourceAddress, Vault>;
type ApprovalMap = PrehashedMap<(ResourceAddress, NonFungibleAddress), Amount>;

#[template]
mod account_template {
    use super::*;

    pub struct Account {
        #[cbor(with = "indexmap_codec")]
        vaults: VaultMap,
        #[cbor(with = "indexmap_codec", default)]
        /// Approvals for other accounts to withdraw from this account. The key is a tuple of (approved resource,
        /// required spender_badge), and the value is the approved amount.
        approvals: ApprovalMap,
    }

    impl Account {
        pub fn create(
            public_key_token: NonFungibleAddress,
            owner_rule: Option<OwnerRule>,
            access_rules: Option<AccessRules>,
            bucket: Option<Bucket>,
        ) -> Component<Account> {
            // extract the public key from the token
            // we only allow tokens that correspond to public keys
            let public_key = public_key_token
                .to_public_key()
                .unwrap_or_else(|| panic!("public_key_token is not a valid public key: {}", public_key_token));

            // The owner of this account is either provided explicitly or defaults to the provided public key.
            let owner_rule = owner_rule.unwrap_or(OwnerRule::ByPublicKey(public_key));

            let access_rules = access_rules.unwrap_or(
                // By default, allow deposits from anyone
                AccessRules::new()
                    .add_method_rule("balance", rule!(allow_all))
                    .add_method_rule("get_balances", rule!(allow_all))
                    .add_method_rule("deposit", rule!(allow_all))
                    .add_method_rule("withdraw_approved", rule!(allow_all))
                    // By default, only the owner of the token will be able to withdraw funds from the account
                    .default(rule!(deny_all)),
            );

            // add the funds from the (optional) bucket
            let mut vaults = VaultMap::default();
            if let Some(b) = bucket {
                vaults.insert(b.resource_address(), Vault::from_bucket(b));
            }

            Component::new(Self {
                vaults,
                approvals: ApprovalMap::default(),
            })
            .with_access_rules(access_rules)
            .with_public_key_address(public_key)
            .with_owner_rule(owner_rule)
            .create()
        }

        pub fn balance(&self, resource: ResourceAddress) -> Amount {
            self.vaults
                .get(&resource)
                .map(|v| v.balance())
                .unwrap_or_else(Amount::zero)
        }

        /// Only applies to confidential resources. Returns the number of commitments in the vault.
        pub fn confidential_commitment_count(&self, resource: ResourceAddress) -> u32 {
            self.get_vault(resource).commitment_count()
        }

        pub fn withdraw(&mut self, resource: ResourceAddress, amount: Amount) -> Bucket {
            // An event is emitted by the vault.withdraw method
            let v = self.get_vault_mut(resource);
            v.withdraw(amount)
        }

        pub fn withdraw_all(&mut self, resource: ResourceAddress) -> Bucket {
            let v = self.get_vault_mut(resource);
            v.withdraw_all()
        }

        pub fn withdraw_non_fungible(&mut self, resource: ResourceAddress, nf_id: NonFungibleId) -> Bucket {
            // An event is emitted by the vault.withdraw_non_fungibles method
            let v = self.get_vault_mut(resource);
            v.withdraw_non_fungibles([nf_id])
        }

        pub fn withdraw_many_non_fungibles(&mut self, resource: ResourceAddress, nf_ids: Vec<NonFungibleId>) -> Bucket {
            // An event is emitted by the vault.withdraw_non_fungibles method
            let v = self.get_vault_mut(resource);
            v.withdraw_non_fungibles(nf_ids)
        }

        pub fn withdraw_confidential(
            &mut self,
            resource: ResourceAddress,
            withdraw_proof: ConfidentialWithdrawProof,
        ) -> Bucket {
            // An event is emitted by the vault.withdraw_confidential method
            let v = self.get_vault_mut(resource);
            v.withdraw_confidential(withdraw_proof)
        }

        pub fn deposit(&mut self, bucket: Bucket) {
            // Ignore empty buckets
            if bucket.is_empty() {
                bucket.drop_empty();
                return;
            }
            // An event is emitted by the vault.deposit method
            let resource_address = bucket.resource_address();
            let vault_mut = self
                .vaults
                .entry(resource_address)
                .or_insert_with(|| Vault::new_empty(resource_address));
            vault_mut.deposit(bucket);
        }

        fn get_vault(&self, resource: ResourceAddress) -> &Vault {
            self.vaults
                .get(&resource)
                .unwrap_or_else(|| panic!("No vault for resource {}", resource))
        }

        fn get_vault_mut(&mut self, resource: ResourceAddress) -> &mut Vault {
            self.vaults
                .get_mut(&resource)
                .unwrap_or_else(|| panic!("No vault for resource {}", resource))
        }

        pub fn get_balances(&self) -> Vec<(ResourceAddress, Amount)> {
            self.vaults.iter().map(|(k, v)| (*k, v.balance())).collect()
        }

        /// Withdraws funds using the ConfidentialWithdrawProof, and immediately deposits the withdrawal back into the
        /// vault. It will panic if the proof is invalid or the resource type contained in the vault is not
        /// confidential. This is useful for converting confidential tokens into revealed tokens and vice versa.
        pub fn join_confidential(&mut self, resource: ResourceAddress, proof: ConfidentialWithdrawProof) {
            // An event is emitted by the vault.withdraw_confidential and vault.deposit methods
            let vault_mut = self.get_vault_mut(resource);
            let bucket = vault_mut.withdraw_confidential(proof);
            vault_mut.deposit(bucket);
        }

        // Fee methods. These are used to pay fees and satisfy a "duck-typed" interface.

        /// Pay fees from previously revealed stealth resource.
        pub fn pay_fee(&mut self, amount: Amount) {
            self.get_vault_mut(STEALTH_TARI_RESOURCE_ADDRESS).pay_fee(amount);
        }

        /// Reveal stealth tokens and return the revealed bucket to pay fees.
        pub fn pay_fee_stealth(&mut self, transfer: StealthTransferStatement) {
            self.get_vault_mut(STEALTH_TARI_RESOURCE_ADDRESS)
                .pay_fee_stealth(transfer);
        }

        pub fn create_proof_for_resource(&mut self, resource: ResourceAddress) -> Proof {
            let v = self.get_vault_mut(resource);
            v.create_proof()
        }

        pub fn create_proof_by_non_fungible(&mut self, nft: NonFungibleAddress) -> Proof {
            self.create_proof_by_non_fungible_ids(*nft.resource_address(), vec![nft.id().clone()])
        }

        pub fn create_proof_by_non_fungible_ids(
            &mut self,
            resource: ResourceAddress,
            ids: Vec<NonFungibleId>,
        ) -> Proof {
            let v = self.get_vault_mut(resource);
            v.create_proof_by_non_fungible_ids(ids.into_iter().collect())
        }

        pub fn create_proof_by_amount(&mut self, resource: ResourceAddress, amount: Amount) -> Proof {
            let v = self.get_vault_mut(resource);
            v.create_proof_by_amount(amount)
        }

        pub fn create_ownership_proof(&mut self) -> Proof {
            ComponentManager::current().get_owner_proof()
        }

        // Approval methods
        /// Called by account owner — sets or updates approval
        pub fn approve(&mut self, spender_badge: NonFungibleAddress, resource: ResourceAddress, amount: Amount) {
            if amount.is_zero() {
                let key = (resource, spender_badge);
                if self.approvals.swap_remove(&key).is_some() {
                    emit_event("revoke_approval", [
                        ("resource", key.0.to_string()),
                        ("spender_badge", key.1.to_string()),
                    ]);
                }
                // Else no-op
            } else {
                emit_event(
                    "approve",
                    metadata!(
                        "spender_badge" => spender_badge.to_string(),
                        "resource" => resource.to_string(),
                        "amount" => amount.to_string(),
                    ),
                );

                self.approvals.insert((resource, spender_badge), amount);
            }
        }

        pub fn revoke_all_approvals(&mut self) {
            self.approvals.clear();
            emit_event("revoke_all_approvals", metadata!());
        }

        pub fn withdraw_approved(&mut self, spender_proof: Proof, resource: ResourceAddress, amount: Amount) -> Bucket {
            let mut badges = spender_proof.get_non_fungibles();
            let Some(badge) = badges.pop_first() else {
                panic!("Proof does not contain any badges");
            };
            let badge_resource = spender_proof.resource_address();
            let badge = NonFungibleAddress::new(badge_resource, badge);

            let approval = self
                .approvals
                .get_mut(&(resource, badge.clone()))
                .unwrap_or_else(|| panic!("No approval for badge {} on resource {}", badge, resource));

            *approval = approval
                .checked_sub(amount)
                .unwrap_or_else(|| panic!("Amount exceeds approval (max: {}, attempted: {})", approval, amount));
            // Clean up zero approvals
            if approval.is_zero() {
                self.approvals.swap_remove(&(resource, badge));
            }

            emit_event("withdraw_approved", [
                ("spender_badge", badge_resource.to_string()),
                ("resource", resource.to_string()),
                ("amount", amount.to_string()),
            ]);

            self.get_vault_mut(resource).withdraw(amount)
        }
    }
}
