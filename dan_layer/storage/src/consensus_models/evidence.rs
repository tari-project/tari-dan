//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use tari_dan_common_types::{committee::CommitteeInfo, SubstateAddress};

use crate::consensus_models::{QcId, SubstateLockType, VersionedSubstateIdLockIntent};

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
                    qc_ids: IndexSet::new(),
                    lock: i.lock_type(),
                })
            })
            .chain(resulting_outputs.into_iter().map(|output| {
                let o = output.borrow();
                (o.to_substate_address(), ShardEvidence {
                    qc_ids: IndexSet::new(),
                    lock: o.lock_type(),
                })
            }))
            .collect()
    }

    pub fn all_addresses_justified(&self) -> bool {
        // TODO: we should check that remote has one QC and local has three
        self.evidence.values().all(|qc_ids| !qc_ids.is_empty())
    }

    pub fn all_input_addresses_justified(&self) -> bool {
        self.evidence
            .values()
            .filter(|e| !e.lock.is_output())
            .all(|qc_ids| !qc_ids.is_empty())
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

    pub fn num_justified_shards(&self) -> usize {
        self.evidence
            .values()
            .filter(|evidence| !evidence.qc_ids.is_empty())
            .count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SubstateAddress, &ShardEvidence)> {
        self.evidence.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SubstateAddress, &mut ShardEvidence)> {
        self.evidence.iter_mut()
    }

    pub fn add_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        for (address, evidence_mut) in self.iter_mut() {
            if committee_info.includes_substate_address(address) {
                evidence_mut.qc_ids.insert(qc_id);
            }
        }

        self
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
        self.evidence.values().flat_map(|e| e.qc_ids.iter())
    }

    /// Add or update substate addresses and locks into Evidence
    pub fn update<I: IntoIterator<Item = (SubstateAddress, SubstateLockType)>>(&mut self, extend: I) -> &mut Self {
        for (substate_address, lock_type) in extend {
            self.evidence
                .entry(substate_address)
                .and_modify(|evidence| evidence.lock = lock_type)
                .or_insert_with(|| ShardEvidence {
                    qc_ids: IndexSet::new(),
                    lock: lock_type,
                });
        }
        self
    }

    /// Merges the other Evidence into this Evidence. If a substate address is present in both, the lock type is
    /// updated to the lock type and the QCs are appended to this instance.
    pub fn merge(&mut self, other: Evidence) -> &mut Self {
        for (substate_address, shard_evidence) in other.evidence {
            let entry = self
                .evidence
                .entry(substate_address)
                .and_modify(|evidence| {
                    evidence.lock = shard_evidence.lock;
                })
                .or_insert_with(|| ShardEvidence {
                    qc_ids: IndexSet::new(),
                    lock: shard_evidence.lock,
                });
            entry.qc_ids.extend(shard_evidence.qc_ids);
        }
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
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub qc_ids: IndexSet<QcId>,
    pub lock: SubstateLockType,
}

impl ShardEvidence {
    pub fn new(qc_ids: IndexSet<QcId>, lock: SubstateLockType) -> Self {
        Self { qc_ids, lock }
    }

    pub fn is_empty(&self) -> bool {
        self.qc_ids.is_empty()
    }

    pub fn contains(&self, qc_id: &QcId) -> bool {
        self.qc_ids.contains(qc_id)
    }

    pub fn insert(&mut self, qc_id: QcId) {
        self.qc_ids.insert(qc_id);
    }
}

impl Display for ShardEvidence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {} QC(s))", self.lock, self.qc_ids.len())
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
