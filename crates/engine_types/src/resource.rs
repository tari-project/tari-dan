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

use std::borrow::Cow;

use ootle_byte_type::FromByteType;
use serde::{Deserialize, Serialize};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArrayError};
use tari_template_lib::{
    resource::TOKEN_SYMBOL,
    types::{
        Amount,
        AuthHook,
        Metadata,
        ResourceType,
        SubstateOwnerRule,
        access_rules::{AccessRule, ResourceAccessRules, ResourceAuthAction},
        crypto::RistrettoPublicKeyBytes,
    },
};

use crate::ownership::Ownership;

#[derive(
    Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen, Serialize, Deserialize, borsh::BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Resource {
    #[n(0)]
    resource_type: ResourceType,
    #[n(1)]
    owner_rule: SubstateOwnerRule,
    #[n(2)]
    access_rules: ResourceAccessRules,
    #[n(3)]
    metadata: Metadata,
    /// The total supply of the resource. None means total_supply tracking is disabled.
    #[n(4)]
    total_supply: Option<Amount>,
    #[n(5)]
    view_key: Option<RistrettoPublicKeyBytes>,
    #[n(6)]
    auth_hook: Option<AuthHook>,
    #[n(7)]
    divisibility: u8,
}

impl Resource {
    pub const fn new(
        resource_type: ResourceType,
        owner_rule: SubstateOwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        view_key: Option<RistrettoPublicKeyBytes>,
        auth_hook: Option<AuthHook>,
        mut divisibility: u8,
        is_total_supply_tracking_enabled: bool,
    ) -> Self {
        // TODO: improve API to make it impossible to set incorrect divisibility
        if resource_type.is_non_fungible() {
            divisibility = 0;
        }

        Self {
            resource_type,
            owner_rule,
            access_rules,
            metadata,
            total_supply: if is_total_supply_tracking_enabled {
                Some(Amount::zero())
            } else {
                None
            },
            divisibility,
            view_key,
            auth_hook,
        }
    }

    pub fn load(
        resource_type: ResourceType,
        owner_rule: SubstateOwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        view_key: Option<RistrettoPublicKeyBytes>,
        auth_hook: Option<AuthHook>,
        divisibility: u8,
        total_supply: Option<Amount>,
    ) -> Self {
        Self {
            resource_type,
            owner_rule,
            access_rules,
            metadata,
            total_supply,
            view_key,
            auth_hook,
            divisibility,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    pub fn owner_rule(&self) -> &SubstateOwnerRule {
        &self.owner_rule
    }

    pub fn owner_key(&self) -> Option<&RistrettoPublicKeyBytes> {
        self.owner_rule.owned_by_public_key()
    }

    pub fn as_ownership(&self) -> Ownership<'_> {
        Ownership {
            owner_rule: Cow::Borrowed(&self.owner_rule),
        }
    }

    pub fn view_key(&self) -> Option<&RistrettoPublicKeyBytes> {
        self.view_key.as_ref()
    }

    /// Converts the view key to a `RistrettoPublicKey`, returning `None` if the view key is not set
    /// or returning an error if the view key is not a canonical compressed representation of a Ristretto public key.
    pub fn to_view_key_public_key(&self) -> Result<Option<RistrettoPublicKey>, ByteArrayError> {
        match self.view_key.as_ref() {
            Some(view_key) => view_key.try_from_byte_type().map(Some),
            None => Ok(None),
        }
    }

    pub fn auth_hook(&self) -> Option<&AuthHook> {
        self.auth_hook.as_ref()
    }

    /// Replaces the resource's authorization hook, or removes it when `auth_hook` is `None`. The caller is
    /// responsible for authorizing the change against
    /// [`ResourceAccessRules::auth_hook_updater`](tari_template_lib::types::access_rules::ResourceAccessRules::auth_hook_updater)
    /// and for validating the hook's signature.
    pub fn set_auth_hook(&mut self, auth_hook: Option<AuthHook>) {
        self.auth_hook = auth_hook;
    }

    /// Writes the resource as a protocol version 0 substate hash preimage: `ResourceAccessRules` without
    /// `auth_hook_updater`, which version 0 resources do not carry.
    pub(crate) fn borsh_serialize_v0<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        // Destructured so that a field added to the struct fails to compile here rather than being silently
        // dropped from the version 0 preimage.
        let Self {
            resource_type,
            owner_rule,
            access_rules,
            metadata,
            total_supply,
            view_key,
            auth_hook,
            divisibility,
        } = self;

        borsh::BorshSerialize::serialize(resource_type, writer)?;
        borsh::BorshSerialize::serialize(owner_rule, writer)?;
        access_rules.borsh_serialize_v0(writer)?;
        borsh::BorshSerialize::serialize(metadata, writer)?;
        borsh::BorshSerialize::serialize(total_supply, writer)?;
        borsh::BorshSerialize::serialize(view_key, writer)?;
        borsh::BorshSerialize::serialize(auth_hook, writer)?;
        borsh::BorshSerialize::serialize(divisibility, writer)
    }

    pub fn access_rules(&self) -> &ResourceAccessRules {
        &self.access_rules
    }

    pub fn set_access_rules(&mut self, access_rules: ResourceAccessRules) {
        self.access_rules = access_rules;
    }

    /// Replaces the access rule for a single resource action. The caller is responsible for
    /// authorizing the change against the field's
    /// [`UpdateRule`](tari_template_lib::types::access_rules::UpdateRule).
    pub fn update_access_rule(&mut self, action: ResourceAuthAction, new_rule: AccessRule) {
        self.access_rules.set_access_rule(action, new_rule);
    }

    /// Returns `true` if the resource has enabled supply tracking, otherwise `false`
    pub fn is_supply_tracking_enabled(&self) -> bool {
        self.total_supply.is_some()
    }

    /// Increases the total supply. This is a no-op if total supply tracking is disabled.
    /// Returns `true` if the total supply was successfully increased or supply tracking is disabled, or `false` if it
    /// would overflow.
    pub fn increase_total_supply(&mut self, amount: Amount) -> bool {
        let Some(supply_mut) = self.total_supply.as_mut() else {
            // Total supply tracking is disabled, this call succeeded
            return true;
        };
        let next_supply = supply_mut.checked_add(amount);
        match next_supply {
            Some(new_supply) => {
                *supply_mut = new_supply;
                true
            },
            None => false,
        }
    }

    /// Decreases the total supply. This is a no-op if total supply tracking is disabled.
    /// Returns `true` if the total supply was successfully decreased or supply tracking is disabled, or `false` if it
    /// would underflow. [`Amount`] is unsigned, so an underflow must be rejected rather than wrapped.
    #[must_use]
    pub fn decrease_total_supply(&mut self, amount: Amount) -> bool {
        let Some(supply_mut) = self.total_supply.as_mut() else {
            // Total supply tracking is disabled, this call succeeded
            return true;
        };
        match supply_mut.checked_sub(amount) {
            Some(new_supply) => {
                *supply_mut = new_supply;
                true
            },
            None => false,
        }
    }

    /// Returns the total supply of the resource, or `None` if total supply tracking is disabled.
    pub fn total_supply(&self) -> Option<Amount> {
        self.total_supply
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.metadata = metadata;
    }

    pub fn token_symbol(&self) -> Option<&str> {
        self.metadata.get(TOKEN_SYMBOL)
    }

    pub fn divisibility(&self) -> u8 {
        self.divisibility
    }
}
