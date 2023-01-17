//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::rust::collections::BTreeSet;
use tari_template_lib::models::{Amount, NftTokenId, ResourceAddress};

/// Instances of a single resource kept in Buckets and Vaults
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, PartialEq)]
pub enum ResourceContainer {
    Fungible {
        address: ResourceAddress,
        amount: Amount,
    },
    NonFungible {
        address: ResourceAddress,
        token_ids: BTreeSet<NftTokenId>,
    },
    // Confidential {
    //     inputs: Vec<Commitment>,
    //     outputs: Vec<Commitment>,
    //     kernels: Vec<Kernel>,
    // },
}

impl ResourceContainer {
    pub fn fungible(address: ResourceAddress, amount: Amount) -> ResourceContainer {
        ResourceContainer::Fungible { address, amount }
    }

    pub fn non_fungible(address: ResourceAddress, token_ids: BTreeSet<NftTokenId>) -> ResourceContainer {
        ResourceContainer::NonFungible { address, token_ids }
    }

    pub fn amount(&self) -> Amount {
        match self {
            ResourceContainer::Fungible { amount, .. } => *amount,
            ResourceContainer::NonFungible { token_ids, .. } => token_ids.len().into(),
        }
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        match self {
            ResourceContainer::Fungible { address, .. } => address,
            ResourceContainer::NonFungible { address, .. } => address,
        }
    }

    pub fn non_fungible_token_ids(&self) -> Option<&BTreeSet<NftTokenId>> {
        match self {
            ResourceContainer::NonFungible { token_ids, .. } => Some(token_ids),
            _ => None,
        }
    }

    pub fn deposit(&mut self, other: ResourceContainer) -> Result<(), ResourceError> {
        if self.resource_address() != other.resource_address() {
            return Err(ResourceError::ResourceAddressMismatch {
                expected: *self.resource_address(),
                actual: *other.resource_address(),
            });
        }

        #[allow(clippy::enum_glob_use)]
        use ResourceContainer::*;
        match (self, other) {
            (
                Fungible { amount, .. },
                Fungible {
                    amount: other_amount, ..
                },
            ) => {
                *amount += other_amount;
            },
            (
                NonFungible { token_ids, .. },
                NonFungible {
                    token_ids: other_token_ids,
                    ..
                },
            ) => {
                token_ids.extend(other_token_ids);
            },
            _ => return Err(ResourceError::FungibilityMismatch),
        }
        Ok(())
    }

    pub fn withdraw(&mut self, amt: Amount) -> Result<ResourceContainer, ResourceError> {
        if !amt.is_positive() || amt.is_zero() {
            return Err(ResourceError::InvariantError(
                "Amount must be positive and non-zero".to_string(),
            ));
        }
        match self {
            ResourceContainer::Fungible { amount, .. } => {
                if amt > *amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: "Bucket contained insufficient funds".to_string(),
                    });
                }
                *amount -= amt;
                Ok(ResourceContainer::fungible(*self.resource_address(), amt))
            },
            ResourceContainer::NonFungible { token_ids, .. } => {
                if amt > token_ids.len().into() {
                    return Err(ResourceError::InsufficientBalance {
                        details: "Bucket contained insufficient tokens".to_string(),
                    });
                }
                let num_to_take = usize::try_from(amt.value())
                    .map_err(|_| ResourceError::OperationNotAllowed(format!("Amount {} too large to withdraw", amt)))?;
                let token_ids = token_ids.iter().take(num_to_take).copied().collect();
                Ok(ResourceContainer::non_fungible(*self.resource_address(), token_ids))
            },
        }
    }

    pub fn withdraw_by_ids(&mut self, ids: &BTreeSet<NftTokenId>) -> Result<ResourceContainer, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw by NFT token id from a fungible resource".to_string(),
            )),
            ResourceContainer::NonFungible { token_ids, .. } => {
                let taken_tokens = ids
                    .iter()
                    .map(|id| {
                        token_ids
                            .take(id)
                            .ok_or(ResourceError::NonFungibleTokenIdNotFound { token: *id })
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ResourceContainer::non_fungible(*self.resource_address(), taken_tokens))
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("Resource fungibility does not match")]
    FungibilityMismatch,
    #[error("Resource addresses do not match: expected:{expected}, actual:{actual}")]
    ResourceAddressMismatch {
        expected: ResourceAddress,
        actual: ResourceAddress,
    },
    #[error("Resource did not contain sufficient balance: {details}")]
    InsufficientBalance { details: String },
    #[error("Invariant error: {0}")]
    InvariantError(String),
    #[error("Operation not allowed: {0}")]
    OperationNotAllowed(String),
    #[error("Non fungible token not found: {token}")]
    NonFungibleTokenIdNotFound { token: NftTokenId },
}
