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

use tari_template_abi::rust::collections::BTreeMap;
use tari_template_lib::prelude::*;

#[template]
mod account_template {
    use super::*;
    pub struct Account {
        // TODO: Lazy key value map/store
        vaults: BTreeMap<ResourceAddress, Vault>,
    }

    impl Account {
        pub fn create(owner_token: NonFungibleAddress) -> Component<Account> {
            Self::internal_create(owner_token, None)
        }

        pub fn create_with_bucket(owner_token: NonFungibleAddress, bucket: Bucket) -> Component<Account> {
            Self::internal_create(owner_token, Some(bucket))
        }

        fn internal_create(owner_token: NonFungibleAddress, bucket: Option<Bucket>) -> Component<Account> {
            // extract the public key from the token
            // we only allow owner tokens that correspond to public keys
            let public_key = owner_token
                .to_public_key()
                .unwrap_or_else(|| panic!("owner_token is not a valid public key: {}", owner_token));

            // the account component will be addressed using the public key
            let component_id = public_key.as_hash();

            // only the owner of the token will be able to withdraw funds from the account
            let withdraw_rule =
                AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require(owner_token.into())));
            let rules = AccessRules::new()
                .add_method_rule("balance", AccessRule::AllowAll)
                .add_method_rule("get_balances", AccessRule::AllowAll)
                .add_method_rule("deposit", AccessRule::AllowAll)
                .add_method_rule("deposit_all", AccessRule::AllowAll)
                .add_method_rule("get_non_fungible_ids", AccessRule::AllowAll)
                .default(withdraw_rule);

            // add the funds from the (optional) bucket
            let mut vaults = BTreeMap::new();
            if let Some(b) = bucket {
                vaults.insert(b.resource_address(), Vault::from_bucket(b));
            }

            Component::new(Self { vaults })
                .with_access_rules(rules)
                .with_component_id(component_id)
                .create()
        }

        // #[access_rule(allow_all)]
        pub fn balance(&self, resource: ResourceAddress) -> Amount {
            self.vaults
                .get(&resource)
                .map(|v| v.balance())
                .unwrap_or_else(Amount::zero)
        }

        pub fn confidential_commitment_count(&self, resource: ResourceAddress) -> u32 {
            self.get_vault(resource).commitment_count()
        }

        // #[access_rule(requires(owner_badge))]
        pub fn withdraw(&mut self, resource: ResourceAddress, amount: Amount) -> Bucket {
            // TODO: clean up hashmap api in emit_event
            emit_event("withdraw", [
                ("amount", amount.to_string()),
                ("resource", resource.to_string()),
            ]);
            let v = self.get_vault_mut(resource);
            v.withdraw(amount)
        }

        // #[access_rules(requires(owner_badge))]
        pub fn withdraw_non_fungible(&mut self, resource: ResourceAddress, nf_id: NonFungibleId) -> Bucket {
            emit_event("withdraw_non_fungible", [
                ("id", nf_id.to_string()),
                ("resource", resource.to_string()),
            ]);
            let v = self.get_vault_mut(resource);
            v.withdraw_non_fungibles([nf_id])
        }

        pub fn withdraw_many_non_fungibles(&mut self, resource: ResourceAddress, nf_ids: Vec<NonFungibleId>) -> Bucket {
            emit_event("withdraw_many_non_fungibles", [
                ("resource", resource.to_string()),
                (
                    "ids",
                    nf_ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(","),
                ),
            ]);
            let v = self.get_vault_mut(resource);
            v.withdraw_non_fungibles(nf_ids)
        }

        // #[access_rules(requires(owner_badge))]
        pub fn withdraw_confidential(
            &mut self,
            resource: ResourceAddress,
            withdraw_proof: ConfidentialWithdrawProof,
        ) -> Bucket {
            emit_event("withdraw_confidential", [
                ("num_inputs", withdraw_proof.inputs.len().to_string()),
                ("resource", resource.to_string()),
            ]);

            let v = self.get_vault_mut(resource);
            v.withdraw_confidential(withdraw_proof)
        }

        // #[access_rules(allow_all)]
        pub fn deposit(&mut self, bucket: Bucket) {
            emit_event("deposit", [
                ("amount", bucket.amount().to_string()),
                ("resource", bucket.resource_address().to_string()),
            ]);
            let resource_address = bucket.resource_address();
            let vault_mut = self
                .vaults
                .entry(resource_address)
                .or_insert_with(|| Vault::new_empty(resource_address));
            vault_mut.deposit(bucket);
        }

        pub fn deposit_all(&mut self, buckets: Vec<Bucket>) {
            for bucket in buckets {
                self.deposit(bucket);
            }
        }

        // #[access_rules(require(owner_badge))]
        pub fn get_non_fungible_ids(&self, resource: ResourceAddress) -> Vec<NonFungibleId> {
            let v = self.get_vault(resource);
            v.get_non_fungible_ids()
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

        pub fn reveal_confidential(&mut self, resource: ResourceAddress, proof: ConfidentialWithdrawProof) -> Bucket {
            emit_event("reveal_confidential", [
                ("num_inputs", proof.inputs.len().to_string()),
                ("resource", resource.to_string()),
            ]);
            let v = self.get_vault_mut(resource);
            v.reveal_confidential(proof)
        }

        pub fn join_confidential(&mut self, resource: ResourceAddress, proof: ConfidentialWithdrawProof) {
            emit_event("join_confidential", [
                ("num_inputs", proof.inputs.len().to_string()),
                ("resource", resource.to_string()),
            ]);
            self.get_vault_mut(resource).join_confidential(proof);
        }

        // Fee methods. These are used to pay fees and satisfy a "duck-typed" interface.

        /// Pay fees from previously revealed confidential resource.
        pub fn pay_fee(&mut self, amount: Amount) {
            emit_event("pay_fee", [("amount", amount.to_string())]);
            self.get_vault_mut(CONFIDENTIAL_TARI_RESOURCE_ADDRESS).pay_fee(amount);
        }

        /// Reveal confidential tokens and return the revealed bucket to pay fees.
        pub fn pay_fee_confidential(&mut self, proof: ConfidentialWithdrawProof) {
            emit_event("pay_fee_confidential", [("num_inputs", proof.inputs.len().to_string())]);
            self.get_vault_mut(CONFIDENTIAL_TARI_RESOURCE_ADDRESS)
                .pay_fee_confidential(proof);
        }

        pub fn create_proof_for_resource(&mut self, resource: ResourceAddress) -> Proof {
            emit_event("create_proof_for_resource", [("resource", resource.to_string())]);
            let v = self.get_vault_mut(resource);
            v.create_proof()
        }

        pub fn create_proof_by_non_fungible_ids(
            &mut self,
            resource: ResourceAddress,
            ids: Vec<NonFungibleId>,
        ) -> Proof {
            emit_event("create_proof_by_non_fungible_ids", [
                ("resource", resource.to_string()),
                ("ids", ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")),
            ]);
            let v = self.get_vault_mut(resource);
            v.create_proof_by_non_fungible_ids(ids.into_iter().collect())
        }

        pub fn create_proof_by_amount(&mut self, resource: ResourceAddress, amount: Amount) -> Proof {
            emit_event("create_proof_by_amount", [
                ("resource", resource.to_string()),
                ("amount", amount.to_string()),
            ]);
            let v = self.get_vault_mut(resource);
            v.create_proof_by_amount(amount)
        }
    }
}
