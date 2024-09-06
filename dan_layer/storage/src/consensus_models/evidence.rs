//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
};

use indexmap::IndexMap;
use log::*;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::{committee::CommitteeInfo, SubstateAddress, SubstateLockType, ToSubstateAddress};

use crate::consensus_models::{QcId, VersionedSubstateIdLockIntent};

const LOG_TARGET: &str = "tari::dan::consensus_models::evidence";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Evidence {
    evidence: IndexMap<SubstateAddress, ShardEvidence>,
}

impl Evidence {
    pub fn empty() -> Self {
        Self {
            evidence: IndexMap::new(),
        }
    }

    pub fn from_inputs_and_outputs<
        I: IntoIterator<Item = BI>,
        BI: Borrow<VersionedSubstateIdLockIntent>,
        O: IntoIterator<Item = BO>,
        BO: Borrow<VersionedSubstateIdLockIntent>,
    >(
        resolved_inputs: I,
        resulting_outputs: O,
    ) -> Self {
        resolved_inputs
            .into_iter()
            .map(|input| {
                let i = input.borrow();
                (i.to_substate_address(), ShardEvidence {
                    prepare_justify: None,
                    accept_justify: None,
                    lock: i.lock_type(),
                })
            })
            .chain(resulting_outputs.into_iter().map(|output| {
                let o = output.borrow();
                (o.to_substate_address(), ShardEvidence {
                    prepare_justify: None,
                    accept_justify: None,
                    lock: o.lock_type(),
                })
            }))
            .collect()
    }

    pub fn all_addresses_justified(&self) -> bool {
        // CASE: all inputs and outputs are accept justified. If they have been accept justified, they have implicitly
        // been prepare justified. This may happen if the local node is only involved in outputs (and therefore
        // sequences using the LocalAccept foreign proposal)
        self.evidence.values().all(|e| e.is_accept_justified())
    }

    pub fn all_input_addresses_prepared(&self) -> bool {
        self.evidence
            .values()
            .filter(|e| !e.lock.is_output())
            // CASE: we use prepare OR accept because inputs can only be accept justified if they were prepared. Prepared
            // may be implicit (null) if the local node is only involved in outputs (and therefore sequences using the LocalAccept
            // foreign proposal)
            .all(|e| e.is_prepare_justified() || e.is_accept_justified())
    }

    pub fn is_committee_output_only(&self, committee_info: &CommitteeInfo) -> bool {
        self.evidence
            .iter()
            .filter(|(addr, _)| committee_info.includes_substate_address(addr))
            .all(|(_, e)| e.lock.is_output())
    }

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }

    pub fn len(&self) -> usize {
        self.evidence.len()
    }

    pub fn get(&self, substate_address: &SubstateAddress) -> Option<&ShardEvidence> {
        self.evidence.get(substate_address)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SubstateAddress, &ShardEvidence)> {
        self.evidence.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SubstateAddress, &mut ShardEvidence)> {
        self.evidence.iter_mut()
    }

    pub fn add_prepare_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        for (address, evidence_mut) in self.evidence_in_committee_iter_mut(committee_info) {
            debug!(
                target: LOG_TARGET,
                "add_prepare_qc_evidence {} {address} QC[{qc_id}] {}",
                committee_info.shard_group(),
                evidence_mut.lock
            );
            evidence_mut.prepare_justify = Some(qc_id);
        }

        self
    }

    pub fn add_accept_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        for (address, evidence_mut) in self.evidence_in_committee_iter_mut(committee_info) {
            debug!(
                target: LOG_TARGET,
                "add_accept_qc_evidence {} {address} QC[{qc_id}] {}",
                committee_info.shard_group(),
                evidence_mut.lock
            );
            evidence_mut.accept_justify = Some(qc_id);
        }

        self
    }

    fn evidence_in_committee_iter_mut<'a>(
        &'a mut self,
        committee_info: &'a CommitteeInfo,
    ) -> impl Iterator<Item = (&'a SubstateAddress, &'a mut ShardEvidence)> {
        self.evidence
            .iter_mut()
            .filter(|(addr, _)| committee_info.includes_substate_address(addr))
    }

    /// Returns an iterator over the substate addresses in this Evidence object.
    /// NOTE: not all substates involved in the final transaction are necessarily included in this Evidence object until
    /// the transaction has reached AllAccepted state.
    pub fn substate_addresses_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.evidence.keys()
    }

    pub fn contains(&self, substate_address: &SubstateAddress) -> bool {
        self.evidence.contains_key(substate_address)
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &QcId> + '_ {
        self.evidence
            .values()
            .flat_map(|e| e.prepare_justify.iter().chain(e.accept_justify.iter()))
    }

    /// Add or update substate addresses and locks into Evidence
    pub fn update<I: IntoIterator<Item = (SubstateAddress, SubstateLockType)>>(&mut self, extend: I) -> &mut Self {
        for (substate_address, lock_type) in extend {
            let maybe_pos = self
                .evidence
                .iter()
                // If the update contains an object (as in ObjectKey) that is already in the evidence, update it without duplicating the object key (inputs and outputs may have the same object key)
                .position(|(address, e)| {
                    // There may up to two matching object_key_bytes, but only if one is an input and the other is an output
                    (lock_type.is_output() == e.lock.is_output()) &&
                        address.object_key_bytes() == substate_address.object_key_bytes()
                });
            match maybe_pos {
                Some(pos) => {
                    if let Some((_, evidence_mut)) = self.evidence.get_index_mut(pos) {
                        evidence_mut.lock = lock_type;
                    }
                },
                None => {
                    self.evidence.insert(substate_address, ShardEvidence {
                        prepare_justify: None,
                        accept_justify: None,
                        lock: lock_type,
                    });
                },
            }
        }
        self.evidence.sort_keys();
        self
    }

    /// Merges the other Evidence into this Evidence. If a substate address is present in both, the lock type is
    /// updated to the lock type and the QCs are appended to this instance.
    pub fn merge(&mut self, other: Evidence) -> &mut Self {
        for (substate_address, shard_evidence) in other.evidence {
            let maybe_pos = self
                .evidence
                .iter()
                // If the update contains an object (as in ObjectKey) that is already in the evidence, update it without duplicating the object key (inputs and outputs may have the same object key with a different substate address version)
                // WHY: because we may not know the exact version yet when we include foreign input evidence. We have to include input evidence to allow foreign shard to sequence the transaction.
                // TODO: maybe we can improve this so that evidence never contains invalid versioning i.e. evidence == what we've pledged at all times
                .position(|(address, e)| {
                    (shard_evidence.lock.is_output() == e.lock.is_output()) &&
                        address.object_key_bytes() == substate_address.object_key_bytes()
                });
            match maybe_pos {
                Some(pos) => {
                    if let Some((_, evidence_mut)) = self.evidence.get_index_mut(pos) {
                        evidence_mut.lock = shard_evidence.lock;
                        if let Some(qc_id) = shard_evidence.prepare_justify {
                            evidence_mut.prepare_justify = Some(qc_id);
                        }
                        if let Some(qc_id) = shard_evidence.accept_justify {
                            evidence_mut.accept_justify = Some(qc_id);
                        }
                    }
                },
                None => {
                    self.evidence.insert(substate_address, shard_evidence);
                },
            }
        }
        self.evidence.sort_keys();
        self
    }
}

impl FromIterator<(SubstateAddress, ShardEvidence)> for Evidence {
    fn from_iter<T: IntoIterator<Item = (SubstateAddress, ShardEvidence)>>(iter: T) -> Self {
        let mut evidence = iter.into_iter().collect::<IndexMap<_, _>>();
        evidence.sort_keys();
        Evidence { evidence }
    }
}

impl Display for Evidence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (substate_address, shard_evidence)) in self.evidence.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", substate_address, shard_evidence)?;
        }
        write!(f, "}}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ShardEvidence {
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub prepare_justify: Option<QcId>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub accept_justify: Option<QcId>,
    pub lock: SubstateLockType,
}

impl ShardEvidence {
    pub fn is_prepare_justified(&self) -> bool {
        self.prepare_justify.is_some()
    }

    pub fn is_accept_justified(&self) -> bool {
        self.accept_justify.is_some()
    }
}

impl Display for ShardEvidence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, ", self.lock)?;
        if let Some(qc_id) = self.prepare_justify {
            write!(f, "Prepare[{}]", qc_id)?;
        } else {
            write!(f, "Prepare[NONE]")?;
        }
        if let Some(qc_id) = self.accept_justify {
            write!(f, ", Accept[{}]", qc_id)?;
        } else {
            write!(f, ", Accept[NONE]")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_substate_address(seed: u8) -> SubstateAddress {
        SubstateAddress::from_bytes(&[seed; SubstateAddress::LENGTH]).unwrap()
    }

    #[test]
    fn it_merges_two_evidences_together() {
        let mut evidence1 = Evidence::empty();
        evidence1.update(vec![
            (seed_substate_address(1), SubstateLockType::Write),
            (seed_substate_address(2), SubstateLockType::Read),
        ]);

        let mut evidence2 = Evidence::empty();
        evidence2.update(vec![
            (seed_substate_address(2), SubstateLockType::Output),
            (seed_substate_address(3), SubstateLockType::Output),
        ]);

        evidence1.merge(evidence2);

        assert_eq!(evidence1.len(), 3);
        assert_eq!(
            evidence1.get(&seed_substate_address(1)).unwrap().lock,
            SubstateLockType::Write
        );
        assert_eq!(
            evidence1.get(&seed_substate_address(2)).unwrap().lock,
            SubstateLockType::Output
        );
        assert_eq!(
            evidence1.get(&seed_substate_address(3)).unwrap().lock,
            SubstateLockType::Output
        );
    }
}
