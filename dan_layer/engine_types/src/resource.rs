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
use tari_bor::{borsh, Decode, Encode};
use tari_template_lib::{
    models::{Amount, Metadata},
    resource::ResourceType,
};

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, PartialEq)]
pub struct Resource {
    resource_type: ResourceType,
    metadata: Metadata,
    total_supply: Amount,
}

impl Resource {
    pub fn new(resource_type: ResourceType, metadata: Metadata) -> Self {
        Self {
            resource_type,
            metadata,
            total_supply: 0.into(),
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    pub fn increase_total_supply(&mut self, amount: Amount) {
        assert!(
            amount.is_positive(),
            "Invariant violation in increase_total_supply: amount must be positive"
        );
        self.total_supply += amount;
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
}
