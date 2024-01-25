//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// Clippy complains about mutable key types in the serde::Deserialize implementation. PublicKey is a safe key type as
// there is no way to actually mutate the compressed value once it is lazily initialized.
#![allow(clippy::mutable_key_type)]

use std::{collections::BTreeMap, iter, mem};

use serde::{Deserialize, Serialize};
use tari_common_types::types::Commitment;
use tari_crypto::tari_utilities::ByteArray;
use tari_template_abi::rust::collections::BTreeSet;
use tari_template_lib::{
    crypto::PedersonCommitmentBytes,
    models::{
        Amount,
        ConfidentialOutputProof,
        ConfidentialWithdrawProof,
        NonFungibleAddress,
        NonFungibleId,
        ResourceAddress,
    },
    prelude::ResourceType,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    confidential::{validate_confidential_proof, validate_confidential_withdraw, ConfidentialOutput},
    substate::SubstateId,
};

/// Instances of a single resource kept in Buckets and Vaults
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum ResourceContainer {
    Fungible {
        address: ResourceAddress,
        amount: Amount,
        locked_amount: Amount,
    },
    NonFungible {
        address: ResourceAddress,
        token_ids: BTreeSet<NonFungibleId>,
        locked_token_ids: BTreeSet<NonFungibleId>,
    },
    Confidential {
        address: ResourceAddress,
        #[cfg_attr(feature = "ts", ts(skip))]
        commitments: BTreeMap<Commitment, ConfidentialOutput>,
        revealed_amount: Amount,
        #[cfg_attr(feature = "ts", ts(skip))]
        locked_commitments: BTreeMap<Commitment, ConfidentialOutput>,
        locked_revealed_amount: Amount,
    },
}

impl ResourceContainer {
    pub fn fungible(address: ResourceAddress, amount: Amount) -> ResourceContainer {
        ResourceContainer::Fungible {
            address,
            amount,
            locked_amount: Amount::zero(),
        }
    }

    pub fn non_fungible(address: ResourceAddress, token_ids: BTreeSet<NonFungibleId>) -> ResourceContainer {
        ResourceContainer::NonFungible {
            address,
            token_ids,
            locked_token_ids: BTreeSet::new(),
        }
    }

    pub fn confidential<I: IntoIterator<Item = (Commitment, ConfidentialOutput)>>(
        address: ResourceAddress,
        commitment: I,
        revealed_amount: Amount,
    ) -> ResourceContainer {
        ResourceContainer::Confidential {
            address,
            commitments: commitment.into_iter().collect(),
            revealed_amount,
            locked_commitments: BTreeMap::new(),
            locked_revealed_amount: Amount::zero(),
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
            commitments: iter::once((validated_proof.output.commitment.clone(), validated_proof.output)).collect(),
            revealed_amount: Amount::zero(),
            locked_commitments: BTreeMap::new(),
            locked_revealed_amount: Amount::zero(),
        })
    }

    pub fn amount(&self) -> Amount {
        match self {
            ResourceContainer::Fungible { amount, .. } => *amount,
            ResourceContainer::NonFungible { token_ids, .. } => token_ids.len().into(),
            ResourceContainer::Confidential { revealed_amount, .. } => *revealed_amount,
        }
    }

    pub fn locked_amount(&self) -> Amount {
        match self {
            ResourceContainer::Fungible { locked_amount, .. } => *locked_amount,
            ResourceContainer::NonFungible { locked_token_ids, .. } => locked_token_ids.len().into(),
            ResourceContainer::Confidential {
                locked_revealed_amount, ..
            } => *locked_revealed_amount,
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

    pub fn non_fungible_token_ids(&self) -> &BTreeSet<NonFungibleId> {
        static EMPTY_BTREE_SET: BTreeSet<NonFungibleId> = BTreeSet::new();
        match self {
            ResourceContainer::NonFungible { token_ids, .. } => token_ids,
            _ => &EMPTY_BTREE_SET,
        }
    }

    pub fn into_non_fungible_ids(self) -> Option<BTreeSet<NonFungibleId>> {
        match self {
            ResourceContainer::NonFungible { token_ids, .. } => Some(token_ids),
            _ => None,
        }
    }

    pub fn child_substates(&self) -> impl Iterator<Item = SubstateId> + '_ {
        self.non_fungible_token_ids()
            .iter()
            .map(|id| SubstateId::NonFungible(NonFungibleAddress::new(*self.resource_address(), id.clone())))
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
        if !amt.is_positive() {
            return Err(ResourceError::InvariantError("Amount must be positive".to_string()));
        }
        match self {
            ResourceContainer::Fungible { amount, .. } => {
                if amt > *amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "Bucket contained insufficient funds. Required: {}, Available: {}",
                            amt, amount
                        ),
                    });
                }
                *amount -= amt;
                Ok(ResourceContainer::fungible(*self.resource_address(), amt))
            },
            ResourceContainer::NonFungible { token_ids, .. } => {
                if amt > token_ids.len().into() {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "Bucket contained insufficient tokens. Required: {}, Available: {}",
                            amt,
                            token_ids.len()
                        ),
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
                        details: format!(
                            "Bucket contained insufficient revealed funds. Required: {}, Available: {}",
                            amt, revealed_amount
                        ),
                    });
                }
                *revealed_amount -= amt;
                Ok(ResourceContainer::confidential(*self.resource_address(), None, amt))
            },
        }
    }

    pub fn recall_all(&mut self) -> Result<ResourceContainer, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } | ResourceContainer::NonFungible { .. } => self.withdraw(self.amount()),
            ResourceContainer::Confidential {
                commitments,
                revealed_amount,
                ..
            } => {
                let amount = *revealed_amount;
                *revealed_amount = Amount::zero();
                let commitments = mem::take(commitments);
                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    commitments,
                    amount,
                ))
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
            ResourceContainer::Confidential {
                commitments,
                revealed_amount,
                ..
            } => {
                let inputs = proof
                    .inputs
                    .iter()
                    .map(|input| {
                        let commitment = Commitment::from_canonical_bytes(input.as_bytes()).map_err(|_| {
                            ResourceError::InvalidConfidentialProof {
                                details: "Invalid input commitment".to_string(),
                            }
                        })?;
                        match commitments.remove(&commitment) {
                            Some(_) => Ok(commitment),
                            None => Err(ResourceError::InvalidConfidentialProof {
                                details: format!(
                                    "withdraw_confidential: input commitment {} not found in resource",
                                    commitment.as_public_key()
                                ),
                            }),
                        }
                    })
                    .collect::<Result<Vec<_>, ResourceError>>()?;

                let validated_proof = validate_confidential_withdraw(&inputs, proof)?;
                if let Some(change) = validated_proof.change_output {
                    if commitments.insert(change.commitment.clone(), change).is_some() {
                        return Err(ResourceError::InvariantError(
                            "Confidential deposit contained duplicate commitment in change commitment".to_string(),
                        ));
                    }
                    *revealed_amount += validated_proof.change_revealed_amount;
                }

                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    Some((validated_proof.output.commitment.clone(), validated_proof.output)),
                    validated_proof.output_revealed_amount,
                ))
            },
        }
    }

    pub fn recall_confidential_commitments(
        &mut self,
        commitments: BTreeSet<PedersonCommitmentBytes>,
        revealed_amount: Amount,
    ) -> Result<ResourceContainer, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw confidential assets from a fungible resource".to_string(),
            )),
            ResourceContainer::NonFungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot withdraw confidential assets from a non-fungible resource".to_string(),
            )),
            ResourceContainer::Confidential {
                commitments: existing_commitments,
                revealed_amount: existing_revealed_amount,
                ..
            } => {
                if *existing_revealed_amount < revealed_amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "recall_confidential_commitments: resource container did not contain enough revealed \
                             funds. Required: {}, Available: {}",
                            revealed_amount, existing_revealed_amount
                        ),
                    });
                }

                *existing_revealed_amount -= revealed_amount;

                let recalled = commitments
                    .iter()
                    .map(|commitment| {
                        let commitment = Commitment::from_canonical_bytes(commitment.as_bytes()).map_err(|_| {
                            ResourceError::InvalidConfidentialProof {
                                details: "Invalid input commitment".to_string(),
                            }
                        })?;
                        let output = existing_commitments.remove(&commitment).ok_or_else(|| {
                            ResourceError::InvalidConfidentialProof {
                                details: format!(
                                    "recall_confidential_commitments: input commitment {} not found in resource",
                                    commitment.as_public_key()
                                ),
                            }
                        })?;
                        Ok((commitment, output))
                    })
                    .collect::<Result<Vec<_>, ResourceError>>()?;

                Ok(ResourceContainer::confidential(
                    *self.resource_address(),
                    recalled,
                    revealed_amount,
                ))
            },
        }
    }

    // TODO: remove this as it is exactly the same as withdraw_confidential
    pub fn reveal_confidential(
        &mut self,
        proof: ConfidentialWithdrawProof,
    ) -> Result<ResourceContainer, ResourceError> {
        self.withdraw_confidential(proof)
    }

    /// Returns all confidential commitments. If the resource is not confidential, None is returned.
    pub fn get_confidential_commitments(&self) -> Option<&BTreeMap<Commitment, ConfidentialOutput>> {
        match self {
            ResourceContainer::Fungible { .. } | ResourceContainer::NonFungible { .. } => None,
            ResourceContainer::Confidential { commitments, .. } => Some(commitments),
        }
    }

    pub fn into_confidential_commitments(self) -> Option<BTreeMap<Commitment, ConfidentialOutput>> {
        match self {
            ResourceContainer::Fungible { .. } | ResourceContainer::NonFungible { .. } => None,
            ResourceContainer::Confidential { commitments, .. } => Some(commitments),
        }
    }

    pub fn lock_all(&mut self) -> Result<ResourceContainer, ResourceError> {
        let resource_address = *self.resource_address();
        match self {
            ResourceContainer::Fungible {
                amount, locked_amount, ..
            } => {
                if *amount == 0 {
                    return Err(ResourceError::InsufficientBalance {
                        details: "lock_all: resource container contained no funds".to_string(),
                    });
                }
                let newly_locked_amount = mem::take(amount);
                *locked_amount += newly_locked_amount;
                Ok(ResourceContainer::fungible(resource_address, newly_locked_amount))
            },
            ResourceContainer::NonFungible {
                token_ids,
                locked_token_ids,
                ..
            } => {
                if token_ids.is_empty() {
                    return Err(ResourceError::InsufficientBalance {
                        details: "lock_all: resource container contained no tokens".to_string(),
                    });
                }
                let newly_locked_token_ids = mem::take(token_ids);
                locked_token_ids.extend(newly_locked_token_ids.iter().cloned());

                Ok(ResourceContainer::non_fungible(
                    resource_address,
                    newly_locked_token_ids,
                ))
            },
            ResourceContainer::Confidential {
                commitments,
                revealed_amount,
                locked_commitments,
                locked_revealed_amount,
                ..
            } => {
                if commitments.is_empty() {
                    return Err(ResourceError::InsufficientBalance {
                        details: "lock_all: resource container contained no commitments".to_string(),
                    });
                }
                let newly_locked_commitments = mem::take(commitments);
                let newly_locked_revealed_amount = *revealed_amount;
                locked_commitments.extend(newly_locked_commitments.iter().map(|(c, o)| (c.clone(), o.clone())));
                *locked_revealed_amount += newly_locked_revealed_amount;

                Ok(ResourceContainer::confidential(
                    resource_address,
                    newly_locked_commitments,
                    newly_locked_revealed_amount,
                ))
            },
        }
    }

    pub fn unlock(&mut self, container: ResourceContainer) -> Result<(), ResourceError> {
        if self.resource_type() != container.resource_type() {
            return Err(ResourceError::ResourceTypeMismatch);
        }
        if self.resource_address() != container.resource_address() {
            return Err(ResourceError::ResourceAddressMismatch {
                expected: *self.resource_address(),
                actual: *container.resource_address(),
            });
        }

        match self {
            ResourceContainer::Fungible {
                amount, locked_amount, ..
            } => {
                if *locked_amount < container.amount() {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "unlock: resource container did not contain enough locked funds. Required: {}, Available: \
                             {}",
                            container.amount(),
                            locked_amount
                        ),
                    });
                }
                *amount += container.amount();
                *locked_amount -= container.amount();
            },
            ResourceContainer::NonFungible {
                token_ids,
                locked_token_ids,
                ..
            } => {
                if locked_token_ids.len() < container.non_fungible_token_ids().len() {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "unlock: resource container did not contain enough locked tokens. Required: {}, \
                             Available: {}",
                            container.non_fungible_token_ids().len(),
                            locked_token_ids.len()
                        ),
                    });
                }
                for token in container.non_fungible_token_ids() {
                    let token = locked_token_ids.take(token).ok_or_else(|| {
                        ResourceError::InvariantError(format!(
                            "unlock: tried to unlock token {token} that was not locked",
                        ))
                    })?;
                    token_ids.insert(token);
                }
            },
            ResourceContainer::Confidential {
                commitments,
                locked_commitments,
                revealed_amount,
                locked_revealed_amount,
                ..
            } => {
                if locked_commitments.len() < container.get_commitment_count() as usize {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "unlock: resource container did not contain enough locked commitments. Required: {}, \
                             Available: {}",
                            container.get_commitment_count(),
                            locked_commitments.len()
                        ),
                    });
                }

                if *locked_revealed_amount < container.amount() {
                    return Err(ResourceError::InvariantError(format!(
                        "unlock: resource container did not contain enough locked revealed amount. Required: {}, \
                         Available: {}",
                        container.amount(),
                        locked_revealed_amount
                    )));
                }

                for (commitment, _) in container.get_confidential_commitments().into_iter().flatten() {
                    let (commitment, output) = locked_commitments.remove_entry(commitment).ok_or_else(|| {
                        ResourceError::InvariantError(
                            "unlock: tried to unlock commitment that was not locked".to_string(),
                        )
                    })?;
                    if commitments.insert(commitment, output).is_some() {
                        return Err(ResourceError::InvariantError(
                            "unlock: container contained duplicate commitment".to_string(),
                        ));
                    }
                }
                *revealed_amount += container.amount();
                *locked_revealed_amount -= container.amount();
            },
        }

        Ok(())
    }

    pub fn lock_by_non_fungible_ids(&mut self, ids: BTreeSet<NonFungibleId>) -> Result<Self, ResourceError> {
        match self {
            ResourceContainer::Fungible { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot lock by NFT token id from a fungible resource".to_string(),
            )),
            ResourceContainer::NonFungible {
                token_ids,
                locked_token_ids,
                ..
            } => {
                let mut newly_locked = BTreeSet::new();
                for id in ids {
                    if let Some(token) = token_ids.take(&id) {
                        newly_locked.insert(token.clone());
                        locked_token_ids.insert(token);
                    } else {
                        return Err(ResourceError::NonFungibleTokenIdNotFound { token: id });
                    }
                }
                Ok(ResourceContainer::non_fungible(*self.resource_address(), newly_locked))
            },
            ResourceContainer::Confidential { .. } => Err(ResourceError::OperationNotAllowed(
                "Cannot lock by NFT token id from a confidential resource".to_string(),
            )),
        }
    }

    pub fn lock_by_amount(&mut self, amount: Amount) -> Result<Self, ResourceError> {
        match self {
            ResourceContainer::Fungible {
                amount: available_amount,
                locked_amount,
                ..
            } => {
                if amount > *available_amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "lock_by_amount: resource container did not contain enough funds. Required: {}, \
                             Available: {}",
                            amount, available_amount
                        ),
                    });
                }
                *available_amount -= amount;
                *locked_amount += amount;
                Ok(ResourceContainer::fungible(*self.resource_address(), amount))
            },
            ResourceContainer::NonFungible {
                token_ids,
                locked_token_ids,
                ..
            } => {
                if amount > token_ids.len().into() {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "lock_by_amount: resource container did not contain enough tokens. Required: {}, \
                             Available: {}",
                            amount,
                            token_ids.len()
                        ),
                    });
                }
                let num_to_take = usize::try_from(amount.value())
                    .map_err(|_| ResourceError::OperationNotAllowed(format!("Amount {} too large to lock", amount)))?;
                let newly_locked_token_ids = (0..num_to_take)
                    .map(|_| {
                        token_ids
                            .pop_first()
                            .expect("Invariant violation: tokens.len() < amount")
                    })
                    .collect::<BTreeSet<_>>();
                locked_token_ids.extend(newly_locked_token_ids.iter().cloned());

                Ok(ResourceContainer::non_fungible(
                    *self.resource_address(),
                    newly_locked_token_ids,
                ))
            },
            ResourceContainer::Confidential {
                revealed_amount,
                locked_revealed_amount,
                ..
            } => {
                if amount > *revealed_amount {
                    return Err(ResourceError::InsufficientBalance {
                        details: format!(
                            "lock_by_amount: resource container did not contain enough revealed funds. Required: {}, \
                             Available: {}",
                            amount, revealed_amount
                        ),
                    });
                }
                *revealed_amount -= amount;
                *locked_revealed_amount += amount;
                Ok(ResourceContainer::confidential(*self.resource_address(), None, amount))
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
