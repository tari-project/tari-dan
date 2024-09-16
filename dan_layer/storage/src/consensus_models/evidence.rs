//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use indexmap::IndexMap;
use log::*;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::{
    committee::CommitteeInfo,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    SubstateLockType,
    ToSubstateAddress,
};
use tari_engine_types::serde_with;

use crate::consensus_models::{QcId, VersionedSubstateIdLockIntent};

const LOG_TARGET: &str = "tari::dan::consensus_models::evidence";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Evidence {
    // Serialize JSON as an array of objects since ShardGroup is a non-string key
    #[serde(with = "serde_with::vec")]
    #[cfg_attr(feature = "ts", ts(type = "Array<[any, any]>"))]
    evidence: IndexMap<ShardGroup, ShardGroupEvidence>,
}

impl Evidence {
    pub fn empty() -> Self {
        Self {
            evidence: IndexMap::new(),
        }
    }

    pub fn from_inputs_and_outputs<
        I: IntoIterator<Item = VersionedSubstateIdLockIntent>,
        O: IntoIterator<Item = VersionedSubstateIdLockIntent>,
    >(
        num_preshards: NumPreshards,
        num_committees: u32,
        resolved_inputs: I,
        resulting_outputs: O,
    ) -> Self {
        let mut evidence = IndexMap::<ShardGroup, ShardGroupEvidence>::new();

        for obj in resolved_inputs.into_iter().chain(resulting_outputs) {
            let substate_address = obj.to_substate_address();
            let sg = substate_address.to_shard_group(num_preshards, num_committees);
            let shard_evidence = evidence.entry(sg).or_default();
            shard_evidence.substates.insert(substate_address, obj.lock_type());
            shard_evidence.substates.sort_keys();
        }

        Evidence { evidence }
    }

    pub fn all_addresses_accepted(&self) -> bool {
        // CASE: all inputs and outputs are accept justified. If they have been accept justified, they have implicitly
        // been prepare justified. This may happen if the local node is only involved in outputs (and therefore
        // sequences using the LocalAccept foreign proposal)
        self.evidence.values().all(|e| e.is_accept_justified())
    }

    pub fn all_inputs_prepared(&self) -> bool {
        self.evidence
            .values()
            // CASE: we use prepare OR accept because inputs can only be accept justified if they were prepared. Prepared
            // may be implicit (null) if the local node is only involved in outputs (and therefore sequences using the LocalAccept
            // foreign proposal)
            .all(|e| {
                if e.is_prepare_justified() || e.is_accept_justified() {
                    true
                } else {
                    // TODO: we should only include input evidence in transactions, so we would only need to check justifies
                    // At this point output-only shards may not be justified
                    e.substates.values().all(|lock| lock.is_output())
                }
            })
    }

    pub fn is_committee_output_only(&self, committee_info: &CommitteeInfo) -> bool {
        self.evidence
            .iter()
            .filter(|(sg, _)| committee_info.shard_group() == **sg)
            .flat_map(|(_, e)| e.substates().values())
            .all(|lock| lock.is_output())
    }

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }

    pub fn len(&self) -> usize {
        self.evidence.len()
    }

    pub fn get(&self, shard_group: &ShardGroup) -> Option<&ShardGroupEvidence> {
        self.evidence.get(shard_group)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ShardGroup, &ShardGroupEvidence)> {
        self.evidence.iter()
    }

    pub fn contains(&self, shard_group: &ShardGroup) -> bool {
        self.evidence.contains_key(shard_group)
    }

    pub fn add_prepare_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        for evidence_mut in self.evidence_in_committee_iter_mut(committee_info) {
            debug!(
                target: LOG_TARGET,
                "add_prepare_qc_evidence {} QC[{qc_id}]",
                committee_info.shard_group(),
            );
            evidence_mut.prepare_qc = Some(qc_id);
        }

        self
    }

    pub fn add_accept_qc_evidence(&mut self, committee_info: &CommitteeInfo, qc_id: QcId) -> &mut Self {
        for evidence_mut in self.evidence_in_committee_iter_mut(committee_info) {
            debug!(
                target: LOG_TARGET,
                "add_accept_qc_evidence {} QC[{qc_id}]",
                committee_info.shard_group(),
            );
            evidence_mut.accept_qc = Some(qc_id);
        }

        self
    }

    fn evidence_in_committee_iter_mut<'a>(
        &'a mut self,
        committee_info: &'a CommitteeInfo,
    ) -> impl Iterator<Item = &'a mut ShardGroupEvidence> {
        self.evidence
            .iter_mut()
            .filter(|(sg, _)| committee_info.shard_group() == **sg)
            .map(|(_, e)| e)
    }

    /// Returns an iterator over the substate addresses in this Evidence object.
    /// NOTE: not all substates involved in the final transaction are necessarily included in this Evidence object until
    /// the transaction has reached AllAccepted state.
    pub fn substate_addresses_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.evidence.values().flat_map(|e| e.substates.keys())
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &QcId> + '_ {
        self.evidence
            .values()
            .flat_map(|e| e.prepare_qc.iter().chain(e.accept_qc.iter()))
    }

    pub fn add_shard_group_evidence(
        &mut self,
        shard_group: ShardGroup,
        address: SubstateAddress,
        lock_type: SubstateLockType,
    ) -> &mut Self {
        let entry = self.evidence.entry(shard_group).or_default();
        entry.substates.insert_sorted(address, lock_type);
        self
    }

    /// Add or update shard groups, substates and locks into Evidence. Existing prepare/accept QC IDs are not changed.
    pub fn update(&mut self, other: &Evidence) -> &mut Self {
        for (sg, evidence) in other.iter() {
            let evidence_mut = self.evidence.entry(*sg).or_default();
            evidence_mut
                .substates
                .extend(evidence.substates.iter().map(|(addr, lock)| (*addr, *lock)));
            evidence_mut.sort_substates();
        }
        self.evidence.sort_keys();
        self
    }

    /// Merges the other Evidence into this Evidence.
    pub fn merge(&mut self, other: Evidence) -> &mut Self {
        for (sg, evidence) in other.evidence {
            let evidence_mut = self.evidence.entry(sg).or_default();
            evidence_mut.substates.extend(evidence.substates);
            evidence_mut.sort_substates();
            if let Some(qc_id) = evidence.prepare_qc {
                evidence_mut.prepare_qc = Some(qc_id);
            }
            if let Some(qc_id) = evidence.accept_qc {
                evidence_mut.accept_qc = Some(qc_id);
            }
        }
        self.evidence.sort_keys();
        self
    }
}

impl FromIterator<(ShardGroup, ShardGroupEvidence)> for Evidence {
    fn from_iter<T: IntoIterator<Item = (ShardGroup, ShardGroupEvidence)>>(iter: T) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ShardGroupEvidence {
    substates: IndexMap<SubstateAddress, SubstateLockType>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    prepare_qc: Option<QcId>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    accept_qc: Option<QcId>,
}

impl ShardGroupEvidence {
    pub fn is_prepare_justified(&self) -> bool {
        self.prepare_qc.is_some()
    }

    pub fn is_accept_justified(&self) -> bool {
        self.accept_qc.is_some()
    }

    pub fn substates(&self) -> &IndexMap<SubstateAddress, SubstateLockType> {
        &self.substates
    }

    pub fn sort_substates(&mut self) {
        self.substates.sort_keys();
    }

    pub fn contains(&self, substate_address: &SubstateAddress) -> bool {
        self.substates.contains_key(substate_address)
    }
}

impl Display for ShardGroupEvidence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (substate_address, lock)) in self.substates.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", substate_address, lock)?;
        }
        write!(f, "}}")?;
        if let Some(qc_id) = self.prepare_qc {
            write!(f, " Prepare[{}]", qc_id)?;
        } else {
            write!(f, " Prepare[NONE]")?;
        }
        if let Some(qc_id) = self.accept_qc {
            write!(f, " Accept[{}]", qc_id)?;
        } else {
            write!(f, " Accept[NONE]")?;
        }
        Ok(())
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
        let sg1 = ShardGroup::new(0, 1);
        let sg2 = ShardGroup::new(2, 3);
        let sg3 = ShardGroup::new(4, 5);

        let mut evidence1 = Evidence::empty();
        evidence1.add_shard_group_evidence(sg1, seed_substate_address(1), SubstateLockType::Write);
        evidence1.add_shard_group_evidence(sg1, seed_substate_address(2), SubstateLockType::Read);

        let mut evidence2 = Evidence::empty();
        evidence2.add_shard_group_evidence(sg1, seed_substate_address(2), SubstateLockType::Output);
        evidence2.add_shard_group_evidence(sg2, seed_substate_address(3), SubstateLockType::Output);
        evidence2.add_shard_group_evidence(sg3, seed_substate_address(4), SubstateLockType::Output);

        evidence1.merge(evidence2);

        assert_eq!(evidence1.len(), 3);
        assert_eq!(
            *evidence1
                .get(&sg1)
                .unwrap()
                .substates
                .get(&seed_substate_address(1))
                .unwrap(),
            SubstateLockType::Write
        );
        assert_eq!(
            *evidence1
                .get(&sg1)
                .unwrap()
                .substates
                .get(&seed_substate_address(2))
                .unwrap(),
            SubstateLockType::Output
        );
        assert_eq!(
            *evidence1
                .get(&sg1)
                .unwrap()
                .substates
                .get(&seed_substate_address(2))
                .unwrap(),
            SubstateLockType::Output
        );
    }
}
