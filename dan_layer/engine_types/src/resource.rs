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

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_template_lib::{
    auth::{OwnerRule, Ownership, ResourceAccessRules},
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, Metadata},
    resource::{ResourceType, TOKEN_SYMBOL},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Resource {
    resource_type: ResourceType,
    owner_rule: OwnerRule,
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    owner_key: Option<RistrettoPublicKeyBytes>,
    access_rules: ResourceAccessRules,
    metadata: Metadata,
    total_supply: Amount,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    view_key: Option<PublicKey>,
}

impl Resource {
    pub fn new(
        resource_type: ResourceType,
        owner_key: Option<RistrettoPublicKeyBytes>,
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        view_key: Option<PublicKey>,
    ) -> Self {
        Self {
            resource_type,
            owner_rule,
            owner_key,
            access_rules,
            metadata,
            total_supply: 0.into(),
            view_key,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    pub fn owner_rule(&self) -> &OwnerRule {
        &self.owner_rule
    }

    pub fn owner_key(&self) -> Option<&RistrettoPublicKeyBytes> {
        self.owner_key.as_ref()
    }

    pub fn as_ownership(&self) -> Ownership<'_> {
        Ownership {
            owner_key: self.owner_key.as_ref(),
            owner_rule: &self.owner_rule,
        }
    }

    pub fn view_key(&self) -> Option<&PublicKey> {
        self.view_key.as_ref()
    }

    pub fn access_rules(&self) -> &ResourceAccessRules {
        &self.access_rules
    }

    pub fn set_access_rules(&mut self, access_rules: ResourceAccessRules) {
        self.access_rules = access_rules;
    }

    pub fn increase_total_supply(&mut self, amount: Amount) -> bool {
        assert!(
            amount.is_positive(),
            "Invariant violation in increase_total_supply: amount must be positive"
        );
        self.total_supply.checked_add(amount).map_or(false, |new_total| {
            self.total_supply = new_total;
            true
        })
    }

    /// Decreases the total supply.
    ///
    /// ## Panics
    /// Panics if the amount is not positive or if the amount is greater than the total supply.
    pub fn decrease_total_supply(&mut self, amount: Amount) {
        assert!(
            amount.is_positive(),
            "Invariant violation in decrease_total_supply: amount must be positive"
        );
        assert!(
            self.total_supply >= amount,
            "Invariant violation in decrease_total_supply: decrease total supply by more than total supply"
        );
        self.total_supply -= amount;
    }

    pub fn total_supply(&self) -> Amount {
        self.total_supply
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn token_symbol(&self) -> Option<&str> {
        self.metadata.get(TOKEN_SYMBOL).map(|s| s.as_str())
    }
}
