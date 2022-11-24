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
use tari_template_lib::models::{Amount, Metadata, ResourceAddress};

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub struct Resource {
    resource_address: ResourceAddress,
    state: ResourceState,
    metadata: Metadata,
}

impl Resource {
    pub fn fungible(resource_address: ResourceAddress, amount: Amount, metadata: Metadata) -> Self {
        Self {
            resource_address,
            state: ResourceState::Fungible { amount },
            metadata,
        }
    }

    pub fn non_fungible(resource_address: ResourceAddress, token_ids: Vec<u64>, metadata: Metadata) -> Self {
        Self {
            resource_address,
            state: ResourceState::NonFungible { token_ids },
            metadata,
        }
    }

    pub fn amount(&self) -> Amount {
        match &self.state {
            ResourceState::Fungible { amount } => *amount,
            ResourceState::NonFungible { token_ids } => token_ids.len().into(),
        }
    }

    pub fn address(&self) -> ResourceAddress {
        self.resource_address
    }

    pub fn non_fungible_token_ids(&self) -> Vec<u64> {
        match &self.state {
            ResourceState::NonFungible { token_ids } => token_ids.clone(),
            _ => Vec::new(),
        }
    }

    pub fn deposit(&mut self, other: Resource) -> Result<(), ResourceError> {
        if self.resource_address != other.resource_address {
            return Err(ResourceError::ResourceAddressMismatch);
        }

        #[allow(clippy::enum_glob_use)]
        use ResourceState::*;
        match (&mut self.state, other.state) {
            (Fungible { amount }, Fungible { amount: other_amount }) => {
                *amount += other_amount;
            },
            (
                NonFungible { token_ids },
                NonFungible {
                    token_ids: other_token_ids,
                },
            ) => {
                token_ids.extend(other_token_ids);
            },
            _ => return Err(ResourceError::FungibilityMismatch),
        }
        Ok(())
    }

    pub fn withdraw(&mut self, amt: Amount) -> Result<Resource, ResourceError> {
        match &mut self.state {
            ResourceState::Fungible { amount } => {
                if amt > *amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: "Bucket contained insufficient funds".to_string(),
                    });
                }
                *amount -= amt;
                Ok(Resource::fungible(self.resource_address, amt, Metadata::default()))
            },
            // TODO: implement an amount type that can apply to both fungible and non fungible resources
            ResourceState::NonFungible { .. } => todo!(),
        }
    }
}

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub enum ResourceState {
    Fungible { amount: Amount },
    NonFungible { token_ids: Vec<u64> },
    // Confidential {
    //     inputs: Vec<Commitment>,
    //     outputs: Vec<Commitment>,
    //     kernels: Vec<Kernel>,
    // },
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("Resource fungibility does not match")]
    FungibilityMismatch,
    #[error("Resource addresses do not match")]
    ResourceAddressMismatch,
    #[error("Resource did not contain sufficient balance: {details}")]
    InsufficientBalance { details: String },
}
