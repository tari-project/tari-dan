//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, Decode, Encode};
use tari_common_types::types::{BulletRangeProof, Commitment, PublicKey};
use tari_template_abi::rust::collections::BTreeSet;
use tari_template_lib::{
    models::{Amount, ConfidentialProof, ConfidentialWithdrawProof, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
};

use crate::{confidential_validation::validate_confidential_proof, confidential_withdraw::check_confidential_withdraw};

/// Instances of a single resource kept in Buckets and Vaults
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, PartialEq)]
pub enum ResourceContainer {
    Fungible {
        address: ResourceAddress,
        amount: Amount,
    },
    NonFungible {
        address: ResourceAddress,
        token_ids: BTreeSet<NonFungibleId>,
    },
    Confidential {
        address: ResourceAddress,
        commitment: PublicKey,
        // TODO: Work out if we need to keep this after VNs have validated it. Bullet proofs can be replayed in
        // transactions.
        range_proof: Option<BulletRangeProof>,
    },
}

impl ResourceContainer {
    pub fn fungible(address: ResourceAddress, amount: Amount) -> ResourceContainer {
        ResourceContainer::Fungible { address, amount }
    }

    pub fn non_fungible(address: ResourceAddress, token_ids: BTreeSet<NonFungibleId>) -> ResourceContainer {
        ResourceContainer::NonFungible { address, token_ids }
    }

    pub fn confidential(
        address: ResourceAddress,
        commitment: PublicKey,
        range_proof: Option<BulletRangeProof>,
    ) -> ResourceContainer {
        ResourceContainer::Confidential {
            address,
            commitment,
            range_proof,
        }
    }

    pub fn validate_confidential(
        address: ResourceAddress,
        proof: ConfidentialProof,
    ) -> Result<ResourceContainer, ResourceError> {
        if proof.change_statement.is_some() {
            return Err(ResourceError::InvalidConfidentialMintWithChange);
        }
        let validated_proof = validate_confidential_proof(proof)?;
        Ok(ResourceContainer::Confidential {
            address,
            commitment: validated_proof.output_commitment.as_public_key().clone(),
            range_proof: Some(validated_proof.output_range_proof),
        })
    }

    pub fn amount(&self) -> Amount {
        match self {
            ResourceContainer::Fungible { amount, .. } => *amount,
            ResourceContainer::NonFungible { token_ids, .. } => token_ids.len().into(),
            ResourceContainer::Confidential { commitment, .. } => {
                // TODO: maybe rather return an option
                // TODO: I think we could have a revealed pool of funds in the resource
                if *commitment == PublicKey::default() {
                    Amount::zero()
                } else {
                    Amount(1)
                }
            },
        }
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        match self {
            ResourceContainer::Fungible { address, .. } => address,
            ResourceContainer::NonFungible { address, .. } => address,
            ResourceContainer::Confidential { address, .. } => address,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        match self {
            ResourceContainer::Fungible { .. } => ResourceType::Fungible,
            ResourceContainer::NonFungible { .. } => ResourceType::NonFungible,
            ResourceContainer::Confidential { .. } => ResourceType::Confidential,
        }
    }

    pub fn non_fungible_token_ids(&self) -> Option<&BTreeSet<NonFungibleId>> {
        match self {
            ResourceContainer::NonFungible { token_ids, .. } => Some(token_ids),
            _ => None,
        }
    }

    pub fn into_non_fungible_ids(self) -> Option<BTreeSet<NonFungibleId>> {
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
            (
                Confidential {
                    commitment,
                    range_proof,
                    ..
                },
                Confidential {
                    commitment: other_commitment,
                    range_proof: other_range_proof,
                    ..
                },
            ) => {
                *commitment = &*commitment + other_commitment;
                *range_proof = other_range_proof;
            },
            _ => return Err(ResourceError::ResourceTypeMismatch),
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
                let taken_tokens = (0..num_to_take)
                    .map(|_| {
                        token_ids
                            .pop_first()
                            .expect("Invariant violation: token_ids.len() < amt")
                    })
                    .collect();

                Ok(ResourceContainer::non_fungible(*self.resource_address(), taken_tokens))
            },
            ResourceContainer::Confidential { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw from a confidential resource by amount".to_string(),
            )),
        }
    }

    pub fn withdraw_by_ids(&mut self, ids: &BTreeSet<NonFungibleId>) -> Result<ResourceContainer, ResourceError> {
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
                            .ok_or_else(|| ResourceError::NonFungibleTokenIdNotFound { token: id.clone() })
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ResourceContainer::non_fungible(*self.resource_address(), taken_tokens))
            },
            ResourceContainer::Confidential { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw by NFT token id from a confidential resource".to_string(),
            )),
        }
    }

    pub fn withdraw_confidential(
        &mut self,
        proof: ConfidentialWithdrawProof,
    ) -> Result<ResourceContainer, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw confidential assets from a fungible resource".to_string(),
            )),
            ResourceContainer::NonFungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw confidential assets from a non-fungible resource".to_string(),
            )),
            ResourceContainer::Confidential { commitment, .. } => {
                let input = Commitment::from_public_key(commitment);
                let validated_proof = check_confidential_withdraw(&input, proof)?;
                *commitment = validated_proof
                    .change_commitment
                    .map(|ch| ch.as_public_key().clone())
                    .unwrap_or_default();

                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    validated_proof.output_commitment.as_public_key().clone(),
                    Some(validated_proof.output_range_proof),
                ))
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("Resource types do not match")]
    ResourceTypeMismatch,
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
    NonFungibleTokenIdNotFound { token: NonFungibleId },
    #[error("Invalid balance proof: {details}")]
    InvalidBalanceProof { details: String },
    #[error("Invalid confidential proof: {details}")]
    InvalidConfidentialProof { details: String },
    #[error("Invalid confidential mint, no change should be specified")]
    InvalidConfidentialMintWithChange,
}
