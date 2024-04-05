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

use std::collections::BTreeSet;

use serde::Serialize;
use tari_bor::to_value;
use tari_template_abi::{call_engine, rust::collections::BTreeMap, EngineOp};

use crate::{
    args::{
        CreateResourceArg,
        InvokeResult,
        MintArg,
        MintResourceArg,
        RecallResourceArg,
        ResourceAction,
        ResourceDiscriminator,
        ResourceGetNonFungibleArg,
        ResourceInvokeArg,
        ResourceRef,
        ResourceUpdateNonFungibleDataArg,
    },
    auth::{OwnerRule, ResourceAccessRules},
    crypto::{PedersonCommitmentBytes, RistrettoPublicKeyBytes},
    models::{
        Amount,
        Bucket,
        ConfidentialOutputStatement,
        Metadata,
        NonFungible,
        NonFungibleId,
        ResourceAddress,
        VaultId,
    },
    prelude::{AuthHook, ResourceType},
};

/// Utility for managing resources inside templates
#[derive(Debug)]
pub struct ResourceManager {
    resource_address: Option<ResourceAddress>,
}

impl ResourceManager {
    /// Returns a new `ResourceManager`
    pub(crate) fn new() -> Self {
        ResourceManager { resource_address: None }
    }

    /// Returns the address of the resource that is being managed
    pub fn get(address: ResourceAddress) -> Self {
        Self {
            resource_address: Some(address),
        }
    }

    fn expect_resource_address(&self) -> ResourceRef {
        let resource_address = self
            .resource_address
            .as_ref()
            .copied()
            .expect("Resource address not set");
        ResourceRef::Ref(resource_address)
    }

    /// Returns the type of the resource that is being managed
    pub fn resource_type(&self) -> ResourceType {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetResourceType,
            args: invoke_args![],
        });
        resp.decode()
            .expect("Resource GetResourceType returned invalid resource type")
    }

    /// Creates a new resource in the Tari network.
    /// Returns the newly created resource address and a `Bucket` with the initial tokens (if minted on creation)
    ///
    /// # Arguments
    ///
    /// * `resource_type` - The type of resource that is being created
    /// * `owner_rule` - Rules that will govern ownership of the resource
    /// * `access_rules` - Rules that will govern access to the resource
    /// * `metadata` - Collection of information used to describe the resource
    /// * `mint_arg` - Specification of the initial tokens that will be minted on resource creation
    pub fn create(
        &self,
        resource_type: ResourceType,
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        mint_arg: Option<MintArg>,
        view_key: Option<RistrettoPublicKeyBytes>,
        authorize_hook: Option<AuthHook>,
    ) -> (ResourceAddress, Option<Bucket>) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: ResourceRef::Resource,
            action: ResourceAction::Create,
            args: invoke_args![CreateResourceArg {
                resource_type,
                owner_rule,
                access_rules,
                metadata,
                mint_arg,
                view_key,
                authorize_hook,
            }],
        });

        resp.decode()
            .expect("[register_non_fungible] Failed to decode ResourceAddress, Option<Bucket> tuple")
    }

    /// Mints new tokens of the confidential resource being managed. Returns a `Bucket` with the newly created tokens.
    ///
    /// It will panic if:
    /// * The resource is not confidential
    /// * The proof is invalid
    /// * The caller doesn't have permissions (via access rules) for minting
    ///
    /// # Arguments
    ///
    /// * `proof` - A zero-knowledge proof that specifies the amount of tokens to be minted and returned back to the
    ///   caller
    pub fn mint_confidential(&self, proof: ConfidentialOutputStatement) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Confidential { proof: Box::new(proof) },
        })
    }

    /// Mints a new non-fungible token of the resource being managed. Returns a `Bucket` with the newly created token.
    ///
    /// It will panic if:
    /// * The resource is not a non-fungible
    /// * The `id` is not unique for the resource
    /// * The caller doesn't have permissions (via access rules) for minting
    ///
    /// # Arguments
    ///
    /// * `id` - The identification of the new non-fungible token. It must be unique for the resource.
    /// * `metadata` - Immutable information used to describe the new token
    /// * `mutable_data` - Initial data that the token will hold and that can potentially be updated in future
    ///   instructions
    pub fn mint_non_fungible<T: Serialize, U: Serialize>(
        &self,
        id: NonFungibleId,
        metadata: &T,
        mutable_data: &U,
    ) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::NonFungible {
                tokens: Some((id, (to_value(metadata).unwrap(), to_value(mutable_data).unwrap())))
                    .into_iter()
                    .collect(),
            },
        })
    }

    /// Mints multiple new non-fungible tokens of the resource being managed.
    /// The `id` of each minted token will be set to random. Returns a `Bucket` with the newly created tokens.
    ///
    /// It will panic if:
    /// * The resource is not a non-fungible
    /// * The caller doesn't have permissions (via access rules) for minting
    ///
    /// # Arguments
    ///
    /// * `metadata` - Immutable information used to describe each new token
    /// * `mutable_data` - Initial data that each token will hold and that can potentially be updated in future
    ///   instructions
    /// * `supply` - The amount of new tokens to be minted
    pub fn mint_many_non_fungible<T: Serialize + ?Sized, U: Serialize + ?Sized>(
        &self,
        metadata: &T,
        mutable_data: &U,
        supply: usize,
    ) -> Bucket {
        let mut tokens = BTreeMap::new();
        let token_data = (to_value(metadata).unwrap(), to_value(mutable_data).unwrap());
        for _ in 0..supply {
            let id = NonFungibleId::random();
            tokens.insert(id, token_data.clone());
        }
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::NonFungible { tokens },
        })
    }

    /// Mints new fungible tokens of the resource being managed.
    /// Returns a `Bucket` with the newly created tokens.
    ///
    /// It will panic if:
    /// * The resource is not fungible
    /// * The caller doesn't have permissions (via access rules) for minting
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount of new tokens to be minted
    pub fn mint_fungible(&self, amount: Amount) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Fungible { amount },
        })
    }

    /// Withdraws all tokens of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// It will panic if the caller doesn't have permissions (via access rules) for recalling
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    pub fn recall_fungible_all(&self, vault_id: VaultId) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Everything,
            vault_id,
        })
    }

    /// Withdraws an amount of tokens of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// It will panic if:
    /// * The resource is not fungible
    /// * The caller doesn't have permissions (via access rules) for recalling
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    /// * `amount` - The amount of tokens to be recalled from the
    pub fn recall_fungible_amount(&self, vault_id: VaultId, amount: Amount) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Fungible { amount },
            vault_id,
        })
    }

    /// Withdraws a single non-fungible token of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// It will panic if:
    /// * The resource is not a non-fungible
    /// * The caller doesn't have permissions (via access rules) for recalling
    /// * The resource does not contain tokens with the ID specified by `token`
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    /// * `token` - The ID of the non-fungible token to be recalled
    pub fn recall_non_fungible(&self, vault_id: VaultId, token: NonFungibleId) -> Bucket {
        self.recall_non_fungibles(vault_id, Some(token).into_iter().collect())
    }

    /// Withdraws multiple non-fungible tokens of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// It will panic if:
    /// * The resource is not a non-fungible
    /// * The caller doesn't have permissions (via access rules) for recalling
    /// * The resource does not contain all the tokens with the IDs specified by `tokens`
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    /// * `tokens` - The IDs of all the non-fungible tokens to be recalled
    pub fn recall_non_fungibles(&self, vault_id: VaultId, tokens: BTreeSet<NonFungibleId>) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::NonFungible { tokens },
            vault_id,
        })
    }

    /// Withdraws an amount of confidential tokens of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// It will panic if:
    /// * The resource is not confidential
    /// * The caller doesn't have permissions (via access rules) for recalling
    /// * `commitments` cointain invalid commitments
    /// * `revealed_amount` is greater than the amount of tokens present in the vault
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    /// * `commitments` - The Pederson commitments of the tokens that are going to be recalled
    /// * `revealed_amount` - The amount of tokens that are going to be recalled
    pub fn recall_confidential(
        &self,
        vault_id: VaultId,
        commitments: BTreeSet<PedersonCommitmentBytes>,
        revealed_amount: Amount,
    ) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Confidential {
                commitments,
                revealed_amount,
            },
            vault_id,
        })
    }

    /// Returns the total supply of tokens for the resource being managed
    pub fn total_supply(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetTotalSupply,
            args: invoke_args![],
        });

        resp.decode().expect("[total_supply] Failed to decode Amount")
    }

    /// Returns the non-fungible token identified by `id`
    /// It will panic if the resource has no tokens identified with `id`
    pub fn get_non_fungible(&self, id: &NonFungibleId) -> NonFungible {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetNonFungible,
            args: invoke_args![ResourceGetNonFungibleArg { id: id.clone() }],
        });

        resp.decode().expect("[get_non_fungible] Failed to decode NonFungible")
    }

    /// Updates the `mutable_data` field of the non-fungible token identified by `id`
    /// It will panic if the resource has no tokens identified with `id`
    pub fn update_non_fungible_data<T: Serialize + ?Sized>(&self, id: NonFungibleId, data: &T) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::UpdateNonFungibleData,
            args: invoke_args![ResourceUpdateNonFungibleDataArg {
                id,
                data: to_value(data).unwrap()
            }],
        });

        resp.decode().expect("[update_non_fungible_data] Failed")
    }

    /// Updates access rules that determine who can operate the resource
    /// It will panic if the caller doesn't have permissions for updating access rules
    pub fn set_access_rules(&self, access_rules: ResourceAccessRules) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::UpdateAccessRules,
            args: invoke_args![access_rules],
        });

        resp.decode().expect("[set_access_rules] Failed")
    }

    fn recall_internal(&self, arg: RecallResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::Recall,
            args: invoke_args![arg],
        });

        let bucket_id = resp.decode().expect("Failed to decode Bucket");
        Bucket::from_id(bucket_id)
    }

    fn mint_internal(&self, arg: MintResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        let bucket_id = resp.decode().expect("Failed to decode Bucket");
        Bucket::from_id(bucket_id)
    }
}
