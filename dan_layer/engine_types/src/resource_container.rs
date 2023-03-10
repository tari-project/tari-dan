//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// Clippy complains about mutable key types in the serde::Deserialize implementation. PublicKey is a safe key type as
// there is no way to actually mutate the compressed value once it is lazily initialized.
#![allow(clippy::mutable_key_type)]

use std::{collections::BTreeMap, iter};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_template_abi::rust::collections::BTreeSet;
use tari_template_lib::{
    models::{Amount, ConfidentialOutputProof, ConfidentialWithdrawProof, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
};
use tari_utilities::ByteArray;

use crate::confidential::{validate_confidential_proof, validate_confidential_withdraw, ConfidentialOutput};

/// Instances of a single resource kept in Buckets and Vaults
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        commitments: BTreeMap<PublicKey, ConfidentialOutput>,
        revealed_amount: Amount,
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
        commitment: Option<(PublicKey, ConfidentialOutput)>,
        revealed_amount: Amount,
    ) -> ResourceContainer {
        ResourceContainer::Confidential {
            address,
            commitments: commitment.into_iter().collect(),
            revealed_amount,
        }
    }

    pub fn validate_confidential_mint(
        address: ResourceAddress,
        proof: ConfidentialOutputProof,
    ) -> Result<ResourceContainer, ResourceError> {
        if proof.change_statement.is_some() {
            return Err(ResourceError::InvalidConfidentialMintWithChange);
        }
        let validated_proof = validate_confidential_proof(&proof)?;
        assert!(
            validated_proof.change_output.is_none(),
            "invariant failed: validate_confidential_proof returned change with no change in input proof"
        );
        Ok(ResourceContainer::Confidential {
            address,
            commitments: iter::once((
                validated_proof.output.commitment.as_public_key().clone(),
                validated_proof.output,
            ))
            .collect(),
            revealed_amount: Amount::zero(),
        })
    }

    pub fn amount(&self) -> Amount {
        match self {
            ResourceContainer::Fungible { amount, .. } => *amount,
            ResourceContainer::NonFungible { token_ids, .. } => token_ids.len().into(),
            ResourceContainer::Confidential { revealed_amount, .. } => *revealed_amount,
        }
    }

    pub fn get_commitment_count(&self) -> u32 {
        match self {
            ResourceContainer::Fungible { .. } => 0,
            ResourceContainer::NonFungible { .. } => 0,
            ResourceContainer::Confidential { commitments, .. } => commitments.len() as u32,
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
                    commitments,
                    revealed_amount,
                    ..
                },
                Confidential {
                    commitments: other_commitments,
                    revealed_amount: other_amount,
                    ..
                },
            ) => {
                for (commit, output) in other_commitments {
                    if commitments.insert(commit, output).is_some() {
                        return Err(ResourceError::InvariantError(
                            "Confidential deposit contained duplicate commitment".to_string(),
                        ));
                    }
                }
                *revealed_amount += other_amount;
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
            ResourceContainer::Confidential { revealed_amount, .. } => {
                if amt > *revealed_amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: "Bucket contained insufficient revealed amount".to_string(),
                    });
                }
                *revealed_amount -= amt;
                Ok(ResourceContainer::confidential(*self.resource_address(), None, amt))
            },
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
            ResourceContainer::Confidential { commitments, .. } => {
                let inputs = proof
                    .inputs
                    .iter()
                    .map(|input| {
                        let commitment =
                            PublicKey::from_bytes(input).map_err(|_| ResourceError::InvalidConfidentialProof {
                                details: "Invalid input commitment".to_string(),
                            })?;
                        match commitments.remove(&commitment) {
                            Some(_) => Ok(commitment),
                            None => Err(ResourceError::InvalidConfidentialProof {
                                details: format!("Input commitment {} found in resource", commitment),
                            }),
                        }
                    })
                    .collect::<Result<Vec<_>, ResourceError>>()?;

                let validated_proof = validate_confidential_withdraw(&inputs, proof)?;
                if let Some(change) = validated_proof.change_output {
                    if commitments
                        .insert(change.commitment.as_public_key().clone(), change)
                        .is_some()
                    {
                        return Err(ResourceError::InvariantError(
                            "Confidential deposit contained duplicate commitment in change commitment".to_string(),
                        ));
                    }
                }

                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    Some((
                        validated_proof.output.commitment.as_public_key().clone(),
                        validated_proof.output,
                    )),
                    Amount::zero(),
                ))
            },
        }
    }

    pub fn reveal_confidential(
        &mut self,
        proof: ConfidentialWithdrawProof,
    ) -> Result<ResourceContainer, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot reveal confidential assets from a fungible resource".to_string(),
            )),
            ResourceContainer::NonFungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot reveal confidential assets from a non-fungible resource".to_string(),
            )),
            ResourceContainer::Confidential { commitments, .. } => {
                let inputs = proof
                    .inputs
                    .iter()
                    .map(|input| {
                        let commitment =
                            PublicKey::from_bytes(input).map_err(|_| ResourceError::InvalidConfidentialProof {
                                details: "Invalid input commitment".to_string(),
                            })?;
                        match commitments.remove(&commitment) {
                            Some(_) => Ok(commitment),
                            None => Err(ResourceError::InvalidConfidentialProof {
                                details: format!("Input commitment {} found in resource", commitment),
                            }),
                        }
                    })
                    .collect::<Result<Vec<_>, ResourceError>>()?;

                let validated_proof = validate_confidential_withdraw(&inputs, proof)?;
                if let Some(change) = validated_proof.change_output {
                    if commitments
                        .insert(change.commitment.as_public_key().clone(), change)
                        .is_some()
                    {
                        return Err(ResourceError::InvariantError(
                            "Confidential reveal contained duplicate commitment in change commitment".to_string(),
                        ));
                    }
                }
                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    Some((
                        validated_proof.output.commitment.as_public_key().clone(),
                        validated_proof.output,
                    )),
                    validated_proof.revealed_amount,
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
