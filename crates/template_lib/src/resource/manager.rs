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

//! This module provides the `ResourceManager` struct, a high-level interface for managing
//! Tari Ootle resources. Resources can be non-private fungible tokens, non-fungible tokens, confidential fungible and
//! stealth fungible.
//!
//! It abstracts common operations like creating resources, minting tokens, recalling tokens from vaults,
//! querying supply, and updating non-fungible metadata.
//!
//! The `ResourceManager` uses engine calls to perform resource operations and enforces
//! access rules and permissions based on the resource configuration.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tari_template_lib::resource::manager::ResourceManager;
//! let resource_manager = ResourceManager::get(my_resource_address);
//! resource_manager.mint_fungible(1000);
//! ```
use minicbor::{CborLen, Decode, Encode};
use tari_bor::to_value;
use tari_template_abi::{
    EngineOp,
    call_engine,
    rust::{
        collections::{BTreeMap, BTreeSet},
        prelude::*,
    },
};
use tari_template_lib_types::{
    AuthHook,
    Metadata,
    NonFungibleId,
    OwnerRule,
    ResourceAddress,
    ResourceInfo,
    UtxoId,
    VaultId,
    access_rules::{AccessRule, ResourceAccessRules, ResourceAuthAction},
    confidential::ConfidentialOutputStatement,
    crypto::CommitmentValueProof,
    stealth::StealthTransferStatement,
};

use crate::{
    args::{
        BurnStealthUtxoArg,
        CreateResourceArg,
        FreezeResourceArg,
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
        SetFreezeConfidentialOutputsArg,
        SetFreezeStealthUtxosArg,
        StealthTransferResourceArg,
        UpdateAccessRuleArg,
        UpdateAuthHookArg,
        VaultFreezeFlags,
    },
    models::{Bucket, BucketId, NonFungible, ResourceAddressAllocation},
    types::{
        Amount,
        ResourceType,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    },
};

/// Provides an interface for various resource operations e.g. Minting, recalling, creating, etc.
///
/// # Example
/// ```rust,ignore
/// use tari_template_lib::resource::manager::ResourceManager;
/// let resource_manager = ResourceManager::get(my_resource_address);
/// resource_manager.mint_fungible(1000);
/// ```
#[derive(Debug, Encode, Decode, CborLen)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct ResourceManager {
    #[n(0)]
    resource_address: ResourceAddress,
}

impl ResourceManager {
    /// Returns the address of the resource that is being managed
    pub fn get(resource_address: ResourceAddress) -> Self {
        Self { resource_address }
    }

    /// Returns the address of the resource that is being managed.
    pub fn resource_address(&self) -> ResourceAddress {
        self.resource_address
    }

    /// Returns the resource type of the resource being managed.
    /// NOTE: this calls `resource_info()` internally, prefer using that if you need the divisibility as well.
    ///
    /// # Panics
    ///
    /// If the resource address does not exist in the runtime context.
    pub fn resource_type(&self) -> ResourceType {
        self.resource_info().resource_type
    }

    /// Returns the divisibility of the resource being managed.
    /// NOTE: this calls `resource_info()` internally, prefer using that if you need the resource type as well.
    ///
    /// # Panics
    ///
    /// If the resource address does not exist in the runtime context.
    pub fn divisibility(&self) -> u8 {
        self.resource_info().divisibility
    }

    /// Returns the resource info of the resource being managed.
    ///
    /// # Panics
    ///
    /// If the resource address does not exist in the runtime context.
    pub fn resource_info(&self) -> ResourceInfo {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::GetResourceInfo,
            args: invoke_args![],
        });
        resp.decode::<ResourceInfo>().expect("Failed to decode ResourceInfo")
    }

    /// Creates a new resource on the Tari network.
    ///
    /// This function registers a new resource, such as a fungible, non-fungible, or confidential asset,
    /// and optionally mints an initial supply. It returns the address of the created resource and,
    /// if tokens were minted, a `Bucket` containing them.
    ///
    /// This method is typically used during component/template initialization to define new tokens or
    /// digital assets with custom access control, metadata, and minting rules.
    ///
    /// # Arguments
    ///
    /// * `resource_type` – The type of resource to create, defined by the [`ResourceType`] enum.
    /// * `owner_rule` – Specifies [`OwnerRule`]s, such as requiring a signature or badge to control the resource.
    /// * `access_rules` – Defines fine-grained permissions ([`ResourceAccessRules`]) for actions like minting, burning,
    ///   or updating data.
    /// * `metadata` – Immutable metadata that describes the resource, such as name, symbol, or description.
    /// * `mint_arg` – Optional initial minting configuration. Must match the `resource_type`.
    /// * `view_key` – (Optional) A [`RistrettoPublicKeyBytes`] used for confidential assets to enable visibility
    ///   control.
    /// * `authorize_hook` – (Optional) An [`AuthHook`] for delegating authorization to another component.
    /// * `address_allocation` – (Optional) A specific [`ResourceAddressAllocation`] used to predefine the address.
    /// * `divisibility` – Number of decimal places allowed. For non-fungible resources, must be 0.
    /// * `is_total_supply_tracking_enabled` – Whether total supply should be tracked and queryable.
    ///
    /// # Returns
    ///
    /// A tuple of:
    /// - [`ResourceAddress`] – The address of the newly created resource.
    /// - [`Option<Bucket>`] – A [`Bucket`] containing the initial minted tokens, if any.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `resource_type` does not match `mint_arg`
    /// - `divisibility` is non-zero for a non-fungible resource
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use tari_template_lib::{
    ///     models::{Amount, Metadata, ResourceAccessRules, ResourceType, OwnerRule, MintArg},
    ///     prelude::*,
    /// };
    ///
    /// let access_rules = ResourceAccessRules::default(); // all actions denied by default
    /// let metadata = Metadata::from_iter([("name", "MyToken"), ("symbol", "MTK")]);
    ///
    /// let (address, bucket) = resource_manager.create(
    ///     ResourceType::Fungible,
    ///     OwnerRule::None,
    ///     access_rules,
    ///     metadata,
    ///     Some(MintArg::Fungible { amount: Amount(1_000_000) }),
    ///     None,
    ///     None,
    ///     None,
    ///     6, // divisible to 6 decimal places
    ///     true,
    /// );
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        resource_type: ResourceType,
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        mint_arg: Option<MintArg>,
        view_key: Option<RistrettoPublicKeyBytes>,
        authorize_hook: Option<AuthHook>,
        address_allocation: Option<ResourceAddressAllocation>,
        divisibility: u8,
        is_total_supply_tracking_enabled: bool,
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
                address_allocation,
                divisibility,
                is_total_supply_tracking_enabled,
            }],
        });

        resp.decode()
            .expect("[register_non_fungible] Failed to decode ResourceAddress, Option<Bucket> tuple")
    }

    /// Mints new tokens for the confidential resource managed by this `ResourceManager`.
    ///
    /// This method accepts a zero-knowledge proof that authorizes the minting of confidential tokens.
    /// Upon success, a [`Bucket`] containing the newly minted tokens is returned to the caller.
    ///
    /// # Arguments
    ///
    /// * `statement` – A [`ConfidentialOutputStatement`] containing the zero-knowledge statement and associated
    ///   metadata. This includes the output and change statements, a range statement, and revealed amounts for output
    ///   and change.
    /// * `value_proofs` – Proves the value of each commitment the statement mints, keyed by commitment. One is required
    ///   per minted commitment if the resource tracks total supply.
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing the newly minted confidential tokens.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    /// - The resource is not of type [`ResourceType::Confidential`]
    /// - The provided statement is invalid or malformed
    /// - The resource tracks total supply and a minted commitment has no valid proof in `value_proofs`
    /// - The caller lacks the required minting permissions, as defined by the resource's [`ResourceAccessRules`]
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let bucket = resource_manager.mint_confidential(statement, value_proofs);
    /// ```
    pub fn mint_confidential(
        &self,
        statement: ConfidentialOutputStatement,
        value_proofs: BTreeMap<PedersenCommitmentBytes, CommitmentValueProof>,
    ) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Confidential {
                statement: Box::new(statement),
                value_proofs,
            },
        })
    }

    /// Mints new revealed tokens for the stealth resource managed by this `ResourceManager`,
    /// returning a [`Bucket`] containing the minted funds.
    ///
    /// # Arguments
    ///
    /// * `amount` – The revealed amount to mint.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    /// - The resource is not of type [`ResourceType::Stealth`]
    /// - The provided amount is negative or zero
    /// - The caller lacks the required minting permissions, as defined by the resource's [`ResourceAccessRules`]
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let bucket = resource_manager.mint_stealth(statement);
    /// ```
    pub fn mint_stealth(&self, amount: Amount) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Stealth { amount },
        })
    }

    /// Mints a new non-fungible token (NFT) for the resource managed by this `ResourceManager`.
    ///
    /// This method creates an NFT with the specified [`NonFungibleId`] and attaches associated metadata and
    /// mutable data. The data must be serializable. On success, a [`Bucket`] containing the newly minted NFT
    /// is returned.
    ///
    /// # Note
    ///
    ///
    ///
    /// # Type Parameters
    ///
    /// * `T` – The type of the static (immutable) metadata associated with the NFT.
    /// * `U` – The type of the mutable data that can later be updated, subject to access rules.
    ///
    /// # Arguments
    ///
    /// * `id` – A unique identifier for the NFT. Must be one of the supported [`NonFungibleId`] variants: `U256`,
    ///   `Uint64`, `Uint32`, or `String`.
    /// * `metadata` – Immutable data describing the NFT (e.g., name, attributes, external links).
    /// * `mutable_data` – Data that can be updated after minting (e.g., usage counters or evolving states).
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing the newly minted non-fungible token.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    /// - The resource is not of type [`ResourceType::NonFungible`]
    /// - Serialization of the metadata or mutable data fails
    /// - The caller does not have minting permissions as defined by [`ResourceAccessRules`]
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let nft = resource_manager.mint_non_fungible(
    ///     NonFungibleId::String("unique_nft_id".to_string()),
    ///     &MyMetadata { name: "My NFT".into() },
    ///     &MyMutableData { views: 0 },
    /// );
    /// ```
    pub fn mint_non_fungible<T: Encode<()>, U: Encode<()>>(
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

    /// Mints multiple non-fungible tokens of the resource being managed, each with the same metadata and mutable data.
    /// Returns a [`Bucket`] containing the newly minted tokens.
    ///
    /// This method generates a number of unique [`NonFungibleId`]s using random identifiers. The `supply` parameter
    /// controls how many tokens are minted.
    ///
    /// # Note
    ///
    /// This method generates random [`NonFungibleId`]s locally without checking for global uniqueness.
    /// While collisions are extremely unlikely due to the large ID space, they are theoretically possible.
    /// Consumers should be aware of this when minting large numbers of NFTs.
    ///
    /// # Arguments
    ///
    /// * `metadata` - A serializable value representing the immutable metadata of the non-fungibles. This is typically
    ///   static information that does not change over the token's lifetime (e.g., name, image URL, category).
    /// * `mutable_data` - A serializable value representing data that can be updated after minting (e.g., usage stats,
    ///   upgrade level).
    /// * `supply` - The number of non-fungible tokens to mint. Each token will have a unique, randomly generated
    ///   [`NonFungibleId`].
    ///
    /// # Access Control
    ///
    /// The caller must satisfy the resource's `mintable` access rule defined in the [`ResourceAccessRules`].
    ///
    /// # Panics
    ///
    /// This function will panic if the resource being managed is not of type [`ResourceType::NonFungible`] or if
    /// serialization of `metadata` or `mutable_data` fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let bucket = manager.mint_many_non_fungible(
    ///     &MyMetadata { name: "Gem".into() },
    ///     &MyMutableData { durability: 100 },
    ///     10,
    /// );
    /// ```
    pub fn mint_many_non_fungible<T: Encode<()> + ?Sized, U: Encode<()> + ?Sized>(
        &self,
        metadata: &T,
        mutable_data: &U,
        supply: u32,
    ) -> Bucket {
        let mut counter = 0;
        self.mint_many_non_fungible_with(metadata, mutable_data, || {
            counter += 1;
            if counter > supply {
                return None;
            }
            Some(NonFungibleId::random())
        })
    }

    /// Mints multiple non-fungible tokens (NFTs) using a custom ID generator.
    ///
    /// This method allows template authors to programmatically define the `NonFungibleId`s
    /// to be used for minting NFTs, by passing a closure that generates new IDs. Each minted
    /// NFT shares the same metadata and mutable data provided to the function.
    ///
    /// The ID generator (`producer`) closure is repeatedly called until it returns `None`.
    /// If the closure returns duplicate IDs, the function will panic to avoid collisions.
    ///
    /// # Note
    /// This function guarantees that the generated IDs are unique within the current minting call.
    /// However, it does **not** ensure global uniqueness across the entire ledger.
    /// If an ID collides with an existing non-fungible token, the mint operation will panic or fail.
    /// Therefore, callers must ensure their ID generation strategy produces globally unique IDs,
    /// typically by using cryptographically secure randomness or a coordinated scheme.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The type of the immutable metadata. Must implement [`Serialize`].
    /// - `U`: The type of the mutable data. Must implement [`Serialize`].
    /// - `F`: A closure that returns `Some(NonFungibleId)` for each new token, or [`None`] to stop the minting process.
    ///
    /// # Arguments
    ///
    /// * `metadata` - A reference to the serializable metadata associated with each token (immutable).
    /// * `mutable_data` - A reference to the serializable mutable data associated with each token.
    /// * `producer` - A closure that generates the next [`NonFungibleId`] to mint. Called repeatedly.
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing all minted NFTs. Each token will have the same metadata and mutable data,
    /// but distinct IDs as returned by the `producer`.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The `producer` closure yields a duplicate `NonFungibleId`
    /// - Serialization of `metadata` or `mutable_data` fails (this is unlikely unless those types are invalid)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut counter = 0;
    /// let bucket = resource_manager.mint_many_non_fungible_with(
    ///     &MyMetadata { name: "Token".into() },
    ///     &MyMutableData { level: 1 },
    ///     || {
    ///         counter += 1;
    ///         if counter > 10 {
    ///             None
    ///         } else {
    ///             Some(NonFungibleId::Uint64(counter))
    ///         }
    ///     },
    /// );
    /// ```
    pub fn mint_many_non_fungible_with<T, U, F>(&self, metadata: &T, mutable_data: &U, mut producer: F) -> Bucket
    where
        T: Encode<()> + ?Sized,
        U: Encode<()> + ?Sized,
        F: FnMut() -> Option<NonFungibleId>,
    {
        let token_data = (to_value(metadata).unwrap(), to_value(mutable_data).unwrap());
        let mut tokens = BTreeMap::new();
        while let Some(id) = producer() {
            if tokens.contains_key(&id) {
                panic!("Non-fungible token with ID {id} already exists in the resource");
            }
            tokens.insert(id, token_data.clone());
        }
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::NonFungible { tokens },
        })
    }

    /// Mints a specified amount of fungible tokens for the resource managed by this `ResourceManager`.
    ///
    /// The minted tokens are returned inside a [`Bucket`], which can be used to hold, transfer, or manipulate the
    /// tokens. The `amount` specifies how many of the smallest indivisible units of the fungible resource should be
    /// minted.
    ///
    /// # Type Parameters
    ///
    /// * `A` – Any type convertible into [`Amount`], representing the quantity of tokens to mint.
    ///
    /// # Arguments
    ///
    /// * `amount` – The quantity of tokens to mint, expressed in the smallest unit of the resource (e.g. microtari,
    ///   satoshis, gwei, etc.).
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing the newly minted fungible tokens.
    ///
    /// # Panics
    ///
    /// This function may panic if:
    /// - The resource is not fungible.
    /// - Minting permissions are not satisfied.
    /// - Internal errors occur during minting.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Mint 1000 units of the fungible token managed by `resource_manager`
    /// let bucket = resource_manager.mint_fungible(1000u64);
    /// ```
    pub fn mint_fungible<A: Into<Amount>>(&self, amount: A) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Fungible { amount: amount.into() },
        })
    }

    fn mint_internal(&self, arg: MintResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        let bucket_id: BucketId = resp.decode().expect("Failed to decode Bucket");
        Bucket::from_id(bucket_id)
    }

    /// Executes a stealth transfer for the resource managed by this [ResourceManager].
    ///
    /// If the [StealthTransferStatement] is valid, and contains revealed outputs, this method will return a
    /// [Bucket] containing the revealed output funds. If the revealed output amount is zero, it will return `None`.
    pub fn stealth_transfer(&self, transfer: StealthTransferStatement) -> Option<Bucket> {
        self.stealth_transfer_internal(transfer, None)
    }

    /// Executes a stealth transfer for the resource managed by this [ResourceManager] consuming the input bucket (if
    /// provided) containing revealed stealth funds.
    ///
    /// If the [StealthTransferStatement] is valid, and contains revealed outputs, this method will return a
    /// [Bucket] containing the revealed output funds. If the revealed output amount is zero, it will return `None`.
    pub fn stealth_transfer_with_opt_input_bucket(
        &self,
        transfer: StealthTransferStatement,
        input_bucket: Option<Bucket>,
    ) -> Option<Bucket> {
        self.stealth_transfer_internal(transfer, input_bucket)
    }

    fn stealth_transfer_internal(
        &self,
        transfer: StealthTransferStatement,
        input_bucket: Option<Bucket>,
    ) -> Option<Bucket> {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::StealthTransfer,
            args: invoke_args![StealthTransferResourceArg {
                transfer,
                input_bucket: input_bucket.map(|b| b.id())
            }],
        });

        resp.decode()
            .expect("[stealth_transfer] Failed to decode Option<Bucket>")
    }

    /// Recalls all tokens of a resource from the specified vault, returning them in a [`Bucket`].
    ///
    /// This method withdraws the entire balance of the resource held in the vault. The caller must have
    /// the necessary permissions as defined by the resource's access rules to perform a recall.
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The identifier of the vault from which all tokens will be recalled.
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing all recalled tokens from the vault.
    ///
    /// # Panics
    ///
    /// This function will panic if the caller lacks the necessary [`OwnerRule`] or [`ResourceAccessRules`] to recall
    /// tokens for the resource.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let vault_id = component.get_user_vault("alice");
    /// let bucket = resource_manager.recall_all(vault_id);
    /// ```
    pub fn recall_all(&self, vault_id: VaultId) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Everything,
            vault_id,
        })
    }

    /// Withdraws an amount of tokens of the resource from the specified vault.
    ///
    /// Allows the user to specify an amount of fungible tokens to be recalled from a specified vault.
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled.
    /// * `amount` - The amount of tokens to be recalled from the vault.
    ///
    /// # Returns
    ///
    /// Returns a [`Bucket`] with the recalled tokens
    ///
    /// # Panics
    ///
    /// It will panic if:
    /// * The [`ResourceType`] is not fungible
    /// * The caller doesn't have the necessary [`OwnerRule`] or [`ResourceAccessRules`] to recall tokens for the
    ///   resource
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let vault_id = component.get_user_vault("bob");
    /// let bucket = resource_manager.recall_fungible_amount(vault_id, 100);
    /// assert_eq!(bucket.amount(), 100);
    /// ```
    pub fn recall_fungible_amount<A: Into<Amount>>(&self, vault_id: VaultId, amount: A) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Fungible { amount: amount.into() },
            vault_id,
        })
    }

    /// Withdraws a single non-fungible token of the resource from the specified vault.
    ///
    /// Method for recalling a single non-fungible token identified by `token` from the specified vault.
    ///
    /// # Arguments
    ///
    /// `vault_id` - The vault whose tokens are going to be recalled
    /// `token` - The ID of the non-fungible token to be recalled.
    ///
    /// # Returns
    ///
    /// Returns a [`Bucket`] with the recalled tokens
    ///
    /// # Panics:
    ///
    /// It will panic if:
    /// * The resource is not of [`ResourceType`] non-fungible
    /// * The caller doesn't have the necessary [`OwnerRule`] or [`ResourceAccessRules`] to recall tokens for the
    ///   resource
    /// * The resource does not contain tokens with the ID specified by `token`
    pub fn recall_non_fungible(&self, vault_id: VaultId, token: NonFungibleId) -> Bucket {
        self.recall_non_fungibles(vault_id, Some(token).into_iter().collect())
    }

    /// Recalls specific non-fungible tokens from the given vault.
    ///
    /// This method withdraws a specified set of non-fungible tokens from a vault.
    ///
    /// # Arguments
    ///
    /// * `vault_id` – The identifier of the vault from which the NFTs should be recalled.
    /// * `tokens` – A set of [`NonFungibleId`]s specifying which tokens to recall.
    ///
    /// # Returns
    ///
    /// A [`Bucket`] containing the recalled non-fungible tokens.
    ///
    /// # Panics
    ///
    /// This function will panic if:
    /// - The resource type of the vault is not [`ResourceType::NonFungible`]
    /// - Any of the requested NFTs do not exist or have been burned
    /// - The caller lacks the necessary permissions sufficient permissions to perform the recall, as defined in the
    ///   resource's [`ResourceAccessRules`] or [`OwnerRule`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let token_ids = btreeset![
    ///     NonFungibleId::U256(1.into()),
    ///     NonFungibleId::U256(2.into()),
    /// ];
    ///
    /// let bucket = resource_manager.recall_non_fungibles(vault_id, token_ids);
    /// ```
    pub fn recall_non_fungibles(&self, vault_id: VaultId, tokens: BTreeSet<NonFungibleId>) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::NonFungible { tokens },
            vault_id,
        })
    }

    /// Withdraws an amount of confidential tokens of the resource from the specified vault.
    /// Returns a `Bucket` with the recalled tokens
    ///
    /// # Arguments
    ///
    /// * `vault_id` - The vault whose tokens are going to be recalled
    /// * `commitments` - The [`PedersenCommitmentBytes`] of the tokens that are going to be recalled
    /// * `revealed_amount` - The amount of tokens that are going to be recalled
    ///
    ///  # Panics
    ///
    /// It will panic if:
    /// * The resource is not of [`ResourceType::Confidential`]
    /// * The caller doesn't have the necessary [`OwnerRule`] or [`ResourceAccessRules`] to recall tokens for the
    ///   resource
    /// * `commitments` contain invalid commitments (invalid Pedersen commitments)
    /// * `revealed_amount` is greater than the amount of tokens present in the vault
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::collections::BTreeSet;
    /// use my_crate::{VaultId, PedersenCommitmentBytes, Amount};
    ///
    /// // Assume you already have a valid `vault_id`
    /// let vault_id = VaultId::from_u64(42);
    /// let mut commitments = BTreeSet::new();
    /// commitments.insert(PedersenCommitmentBytes::from_bytes([0u8; 32]));
    /// let revealed_amount = Amount::from(100u64);
    /// let bucket = engine.recall_confidential(vault_id, commitments, revealed_amount);
    /// assert!(!bucket.is_empty());
    /// ```
    pub fn recall_confidential<A: Into<Amount>>(
        &self,
        vault_id: VaultId,
        commitments: BTreeSet<PedersenCommitmentBytes>,
        revealed_amount: A,
    ) -> Bucket {
        self.recall_internal(RecallResourceArg {
            resource: ResourceDiscriminator::Confidential {
                commitments,
                revealed_amount: revealed_amount.into(),
            },
            vault_id,
        })
    }

    fn recall_internal(&self, arg: RecallResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::Recall,
            args: invoke_args![arg],
        });

        let bucket_id = resp.decode().expect("Failed to decode Bucket");
        Bucket::from_id(bucket_id)
    }

    /// Returns the total supply of tokens for the resource being managed in a [`ResourceManager`] instance.
    ///
    /// If the resource has total supply tracking enabled, the function will return the total supply of tokens.
    ///
    /// If you want to check if the resource has total supply tracking enabled, use
    /// [`total_supply_opt`](Self::total_supply_opt).
    ///
    /// # Panics
    ///
    /// Panics if:
    /// * the resource does not have total supply tracking enabled. You can check this with
    ///   [`total_supply_opt`](Self::total_supply_opt).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let total = resource_manager.total_supply();
    /// println!("Total supply: {}", total);
    /// ```    
    pub fn total_supply(&self) -> Amount {
        self.total_supply_opt()
            .expect("Resource does not have total supply tracking enabled")
    }

    /// Returns the total supply of the resource as an `Option<Amount>`.
    ///
    /// This method invokes the resource engine with a `GetTotalSupply` action
    /// to query the current total supply tracked by the resource. If the resource
    /// does not have total supply tracking enabled, this will return `None`.
    ///
    /// # Returns
    ///
    /// * `Some(Amount)` if total supply tracking is enabled and available.
    /// * `None` if total supply tracking is not enabled.
    ///
    /// # Panics
    ///
    /// Panics if decoding the response into an `Option<Amount>` fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let total_supply = resource_manager.total_supply_opt();
    /// if let Some(amount) = total_supply {
    ///     println!("Total supply is {:?}", amount);
    /// } else {
    ///     println!("Total supply tracking not enabled");
    /// }
    /// ```
    pub fn total_supply_opt(&self) -> Option<Amount> {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::GetTotalSupply,
            args: invoke_args![],
        });

        resp.decode().expect("[total_supply] Failed to decode Amount")
    }

    /// Returns the non-fungible token identified by the given [`NonFungibleId`].
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier of the non-fungible token to retrieve.
    ///
    /// # Returns
    ///
    /// A [`NonFungible`] NFT object containing the token's metadata and mutable data.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    /// - The resource is not of type [`ResourceType::NonFungible`].
    /// - The token with the specified `id` does not exist or has been burned.
    /// - The caller does not have the necessary [`ResourceAccessRules`] or [`OwnerRule`] to access the non-fungible
    ///   token.
    /// # Example
    ///
    ///  ```rust,ignore
    /// let id = NonFungibleId::String("unique_nft_id".to_string());
    /// let nft = resource_manager.get_non_fungible(&id);
    /// println!("NFT Metadata: {:?}", nft.metadata);
    /// println!("NFT Mutable Data: {:?}", nft.mutable_data);
    /// ```
    pub fn get_non_fungible(&self, id: &NonFungibleId) -> NonFungible {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::GetNonFungible,
            args: invoke_args![ResourceGetNonFungibleArg { id: id.clone() }],
        });

        resp.decode().expect("[get_non_fungible] Failed to decode NonFungible")
    }

    /// Updates the mutable data of a non-fungible token (NFT) identified by its `NonFungibleId`.
    ///
    /// This method serializes the provided data and sends it to the engine using the
    /// `UpdateNonFungibleData` action. The data is stored in the mutable portion of the NFT.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of the data being updated. Must implement `Serialize`.
    ///
    /// # Arguments
    ///
    /// * `id` - A `NonFungibleId` identifying the NFT to update. Can be a string, U256, u32, or u64 variant.
    /// * `data` - A reference to the serializable data to store as the NFT's mutable data.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - Serialization of `data` fails,
    /// - The resource address does not exist,
    /// - The resource is not of type [`ResourceType::NonFungible`],
    /// - The engine call fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// # use tari_template_lib::types::NonFungibleId;
    /// # use tari_template_lib::prelude::ResourceManager;
    ///
    /// #[derive(serde::Serialize)]
    /// struct MyMutableData {
    ///     views: u64,
    /// }
    ///
    /// let id = NonFungibleId::String("my_unique_nft".into());
    /// let data = MyMutableData { views: 42 };
    ///
    /// ResourceManager::get("resource_xxx".parse().unwrap()).update_non_fungible_data(id, &data);
    /// ```
    pub fn update_non_fungible_data<T: Encode<()> + ?Sized>(&self, id: NonFungibleId, data: &T) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::UpdateNonFungibleData,
            args: invoke_args![ResourceUpdateNonFungibleDataArg {
                id,
                data: to_value(data).unwrap()
            }],
        });

        resp.decode().expect("[update_non_fungible_data] Failed")
    }

    /// Updates the access rule that gates a single resource action (mint, burn, recall, etc.).
    ///
    /// Authorization is gated by the per-field
    /// [`UpdateRule`](tari_template_lib_types::access_rules::UpdateRule) configured for `action`:
    ///
    /// * [`UpdateRule::Locked`](tari_template_lib_types::access_rules::UpdateRule::Locked): the rule cannot be changed
    ///   by anyone (not even the resource owner).
    /// * [`UpdateRule::Owner`](tari_template_lib_types::access_rules::UpdateRule::Owner): only the resource owner can
    ///   change the rule.
    /// * [`UpdateRule::AccessRule`](tari_template_lib_types::access_rules::UpdateRule::AccessRule): the caller must
    ///   satisfy the embedded [`AccessRule`].
    ///
    /// The updater rule itself is immutable once a resource is created.
    ///
    /// # Arguments
    ///
    /// * `action` - The [`ResourceAuthAction`] whose access rule should be replaced.
    /// * `new_rule` - The new [`AccessRule`] for that action.
    ///
    /// # Panics
    ///
    /// Panics if the caller is not authorized to update the rule for `action`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use tari_template_lib::prelude::{ResourceAuthAction, rule};
    /// resource_manager.update_access_rule(ResourceAuthAction::Mint, rule!(allow_all));
    /// ```
    pub fn update_access_rule(&self, action: ResourceAuthAction, new_rule: AccessRule) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::UpdateAccessRule,
            args: invoke_args![UpdateAccessRuleArg { action, new_rule }],
        });

        resp.decode().expect("[update_access_rule] Failed")
    }

    /// Replaces the resource's authorization hook, or removes it when `auth_hook` is `None`. A resource that
    /// has no hook gains one, so a holder cannot rely on a resource staying hook-free once the updater permits
    /// a change.
    ///
    /// Authorization is gated by the resource's
    /// [`auth_hook_updater`](tari_template_lib_types::access_rules::ResourceAccessRules::auth_hook_updater),
    /// which is [`UpdateRule::Locked`](tari_template_lib_types::access_rules::UpdateRule::Locked) unless the
    /// resource was created with
    /// [`set_auth_hook_updater`](tari_template_lib_types::access_rules::ResourceAccessRules::set_auth_hook_updater).
    ///
    /// The hook being replaced is not consulted, so a hook that denies or panics can still be repaired or
    /// retired. A new hook must satisfy the same signature checks as one supplied at resource creation, and
    /// its component must already exist.
    ///
    /// # Panics
    ///
    /// - The caller does not satisfy the resource's `auth_hook_updater`.
    /// - `auth_hook` names a component or method that does not exist, or a method whose signature is not that of an
    ///   authorization hook.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use tari_template_lib::prelude::AuthHook;
    /// // Repair a broken hook by pointing the resource at a new component.
    /// resource_manager.set_auth_hook(Some(AuthHook::new(new_component, "authorize".try_into().unwrap())));
    /// // Retire the hook entirely.
    /// resource_manager.set_auth_hook(None);
    /// ```
    pub fn set_auth_hook(&self, auth_hook: Option<AuthHook>) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::UpdateAuthHook,
            args: invoke_args![UpdateAuthHookArg { auth_hook }],
        });

        resp.decode().expect("[set_auth_hook] Failed")
    }

    /// Replaces the resource's metadata map.
    ///
    /// The token symbol (`SYMBOL` key) is immutable once set: if the resource already has a symbol,
    /// `metadata` must carry that exact same symbol or the transaction is rejected. If the resource
    /// has no symbol yet, the caller may set one for the first time, and it becomes immutable thereafter.
    ///
    /// # Panics
    ///
    /// - The caller does not have the necessary [`ResourceAccessRules`] or [`OwnerRule`] to update the metadata.
    /// - The new metadata would change or drop an existing token symbol.
    pub fn set_metadata(&self, metadata: Metadata) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::UpdateMetadata,
            args: invoke_args![metadata],
        });

        resp.decode().expect("[set_metadata] Failed")
    }

    /// Freezes all withdrawals, deposits and burns for the specified vault.
    pub fn freeze_vault(&self, vault_id: VaultId) {
        self.set_freeze_vault_flags(vault_id, VaultFreezeFlags::all());
    }

    /// Sets the freeze flags for the specified vault.
    pub fn set_freeze_vault_flags(&self, vault_id: VaultId, flags: VaultFreezeFlags) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::SetVaultFreeze,
            args: invoke_args![FreezeResourceArg { vault_id, flags }],
        });

        resp.decode().expect("SetFreeze failed")
    }

    /// Unfreezes all withdrawals, deposits and burns for the specified vault.
    /// Equivalent to `manager.set_freeze(FreezeFlags::empty())`.
    pub fn unfreeze_vault(&self, vault_id: VaultId) {
        self.set_freeze_vault_flags(vault_id, VaultFreezeFlags::empty());
    }

    /// Freezes the specified stealth UTXOs, preventing them from being spent/transferred.
    pub fn freeze_utxos(&self, utxos: Vec<UtxoId>) {
        self.set_freeze_utxos(utxos, true);
    }

    /// Unfreezes the specified stealth UTXOs, allowing them to be spent/transferred again.
    pub fn unfreeze_utxos(&self, utxos: Vec<UtxoId>) {
        self.set_freeze_utxos(utxos, false);
    }

    fn set_freeze_utxos(&self, utxos: Vec<UtxoId>, freeze: bool) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::SetStealthUtxosFreeze,
            args: invoke_args![SetFreezeStealthUtxosArg { utxos, freeze }],
        });

        resp.decode().expect("SetFreeze failed")
    }

    /// Freezes the specified confidential outputs, preventing them from being spent/transferred.
    pub fn freeze_confidential_outputs(&self, commitments: Vec<PedersenCommitmentBytes>) {
        self.set_freeze_confidential_outputs(commitments, true);
    }

    /// Unfreezes the specified confidential outputs, allowing them to be spent/transferred again.
    pub fn unfreeze_confidential_outputs(&self, commitments: Vec<PedersenCommitmentBytes>) {
        self.set_freeze_confidential_outputs(commitments, false);
    }

    fn set_freeze_confidential_outputs(&self, commitments: Vec<PedersenCommitmentBytes>, freeze: bool) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::SetConfidentialOutputsFreeze,
            args: invoke_args![SetFreezeConfidentialOutputsArg { commitments, freeze }],
        });

        resp.decode().expect("SetConfidentialOutputsFreeze failed")
    }

    /// Burns the stealth UTXO, permanently removing them from circulation.
    /// If total supply tracking is enabled for the resource, a valid `CommitmentValueProof` is required.
    /// NOTE: that this essentially limits burns to the UTXO owner or the secret view key holder regardless of the
    /// access rules (i.e. if burns are limited to an "admin" badge, the admin can only burn funds if they have the
    /// secret view key or burn their own funds). This is a limitation of the current protocol.
    pub fn burn_utxo(&self, utxo_id: UtxoId, value_proof: Option<CommitmentValueProof>) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.resource_address.into(),
            action: ResourceAction::StealthUtxoBurn,
            args: invoke_args![BurnStealthUtxoArg { utxo_id, value_proof }],
        });

        resp.decode().expect("BurnStealthUtxos failed")
    }
}

impl From<ResourceAddress> for ResourceManager {
    fn from(resource_address: ResourceAddress) -> Self {
        Self::get(resource_address)
    }
}
