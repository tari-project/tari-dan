//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use borsh::BorshSerialize;
use indexmap::IndexMap;
use log::*;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_consensus_types::PcId;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{
    LockIntent,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    SubstateLockType,
    SubstateRequirementRef,
    ToSubstateAddress,
    VersionedSubstateId,
    borsh::indexmap as indexmap_borsh,
    displayable::Displayable,
};

use crate::consensus_models::{RequireLockIntentRef, SubstatePledge};

const LOG_TARGET: &str = "tari::ootle::consensus_models::evidence";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Evidence {
    // Serialize JSON as an array of objects since ShardGroup is a non-string key
    #[serde(with = "ootle_serde::map")]
    #[cfg_attr(feature = "ts", ts(type = "Array<[any, any]>"))]
    #[borsh(serialize_with = "indexmap_borsh::serialize")]
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::indexmap_codec")]
    evidence: IndexMap<ShardGroup, ShardGroupEvidence>,
}

impl Evidence {
    pub fn empty() -> Self {
        Self {
            evidence: IndexMap::new(),
        }
    }

    pub fn from_inputs_and_outputs<'a, I, O>(
        num_preshards: NumPreshards,
        num_committees: u32,
        inputs: I,
        outputs: O,
    ) -> Self
    where
        I: IntoIterator<Item = SubstateRequirementRef<'a>>,
        O: IntoIterator<Item = VersionedSubstateId>,
    {
        let mut evidence = Self::empty();

        for obj in inputs {
            // Version does not affect the shard group
            let substate_address = obj.or_zero_version().to_substate_address();
            let sg = substate_address.to_shard_group(num_preshards, num_committees);
            evidence
                .add_shard_group(sg)
                .insert_unpledged_input(obj.substate_id().clone());
        }

        for obj in outputs {
            let substate_address = obj.to_substate_address();
            let sg = substate_address.to_shard_group(num_preshards, num_committees);
            let version = obj.version();
            evidence
                .add_shard_group(sg)
                .insert_output(obj.into_substate_id(), version);
        }

        evidence.evidence.sort_keys();

        evidence
    }

    pub fn from_lock_intents<I, L1>(num_preshards: NumPreshards, num_committees: u32, locks: I) -> Self
    where
        L1: LockIntent + Clone,
        I: IntoIterator<Item = L1>,
    {
        let mut evidence = Self::empty();

        for lock in locks {
            evidence.insert_from_lock_intent(num_preshards, num_committees, lock);
        }

        evidence
    }

    pub fn insert_from_lock_intent<L: LockIntent + Clone>(
        &mut self,
        num_preshards: NumPreshards,
        num_committees: u32,
        lock: L,
    ) -> &mut Self {
        if lock.substate_id().is_global() {
            for sg in num_preshards.all_shard_groups_iter(num_committees) {
                self.add_shard_group(sg).insert_from_lock_intent(lock.clone());
            }
        } else {
            let substate_address = lock.to_substate_address();
            let sg = substate_address.to_shard_group(num_preshards, num_committees);
            self.add_shard_group(sg).insert_from_lock_intent(lock);
        }
        self.evidence.sort_keys();
        self
    }

    pub fn insert_unpledged_from_substate_id(
        &mut self,
        num_preshards: NumPreshards,
        num_committees: u32,
        substate_id: SubstateId,
    ) -> &mut Self {
        if substate_id.is_global() {
            for sg in num_preshards.all_shard_groups_iter(num_committees) {
                self.add_shard_group(sg).insert_unpledged_input(substate_id.clone());
            }
        } else {
            let substate_address = SubstateAddress::from_substate_id(&substate_id, 0);
            let sg = substate_address.to_shard_group(num_preshards, num_committees);
            self.add_shard_group(sg).insert_unpledged_input(substate_id);
        }
        self
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = (&ShardGroup, &SubstateId, &Option<EvidenceInputLockData>)> {
        self.evidence.iter().flat_map(|(sg, evidence)| {
            evidence
                .inputs
                .iter()
                .map(move |(substate_id, lock)| (sg, substate_id, lock))
        })
    }

    pub fn all_outputs_iter(&self) -> impl Iterator<Item = (&ShardGroup, &SubstateId, &u64)> {
        self.evidence.iter().flat_map(|(sg, evidence)| {
            evidence
                .outputs
                .iter()
                .map(move |(substate_id, version)| (sg, substate_id, version))
        })
    }

    pub fn all_shard_groups_accepted(&self) -> bool {
        // CASE: all inputs and outputs are accept justified. If they have been accept justified, they have implicitly
        // been prepare justified. This may happen if the local node is only involved in outputs (and therefore
        // sequences using the LocalAccept foreign proposal)
        self.evidence.values().all(|e| e.is_accept_justified())
    }

    pub fn all_shard_groups_prepared(&self) -> bool {
        self.evidence
            .values()
            // CASE: we use prepare OR accept because inputs can only be accept justified if they were prepared. Prepared
            // may be implicit (null) if the local node is only involved in outputs (and therefore sequences using the LocalAccept
            // foreign proposal)
            .all(|e| e.is_prepare_justified() || e.is_accept_justified())
    }

    pub fn some_shard_groups_prepared(&self) -> bool {
        self.evidence
            .values()
            .any(|e| e.is_prepare_justified() || e.is_accept_justified())
    }

    pub fn all_input_shard_groups_prepared(&self, local_shard_group: ShardGroup) -> bool {
        let local_has_inputs = self.has_inputs(local_shard_group);

        self.evidence
            .values()
            .filter(|e| {
                // CASE: we only require input shard groups to prepare
                !e.inputs().is_empty()
            })
            .all(|e| {
                if local_has_inputs {
                    // Local has inputs: we require prepare justification to continue regardless of accept justification
                    e.is_prepare_justified()
                } else {
                    // Local is Output-only: we consider the input shard groups prepared if we've received prepare OR
                    // accept justification
                    // TODO: Technically, we should only have received accept justification. Is being more lenient a
                    // problem?
                    e.is_prepare_justified() || e.is_accept_justified()
                }
            })
    }

    /// Returns true if all substates in the given shard group are output locks.
    /// This assumes the provided evidence is complete before this is called.
    /// If no evidence is present for the shard group, false is returned.
    pub fn is_committee_output_only(&self, shard_group: ShardGroup) -> bool {
        self.evidence.get(&shard_group).is_some_and(|e| e.inputs().is_empty())
    }

    pub fn output_only_shard_groups_iter(&self) -> impl Iterator<Item = ShardGroup> + '_ {
        self.evidence
            .iter()
            .filter_map(|(sg, e)| if e.inputs().is_empty() { Some(*sg) } else { None })
    }

    pub fn is_committee_input_only(&self, shard_group: ShardGroup) -> bool {
        self.evidence.get(&shard_group).is_none_or(|e| e.outputs().is_empty())
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

    pub fn get_mut(&mut self, shard_group: &ShardGroup) -> Option<&mut ShardGroupEvidence> {
        self.evidence.get_mut(shard_group)
    }

    pub fn abort(&mut self) -> &mut Self {
        for (_, ev_mut) in &mut self.evidence {
            ev_mut.abort();
        }
        self
    }

    pub fn has(&self, shard_group: &ShardGroup) -> bool {
        self.evidence.contains_key(shard_group)
    }

    pub fn has_inputs(&self, shard_group: ShardGroup) -> bool {
        self.evidence.get(&shard_group).is_some_and(|e| !e.inputs.is_empty())
    }

    pub fn has_and_not_empty(&self, shard_group: &ShardGroup) -> bool {
        self.evidence
            .get(shard_group)
            .is_some_and(|e| !e.inputs.is_empty() || !e.outputs.is_empty())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ShardGroup, &ShardGroupEvidence)> {
        self.evidence.iter()
    }

    pub fn contains(&self, shard_group: &ShardGroup) -> bool {
        self.evidence.contains_key(shard_group)
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &PcId> + '_ {
        self.evidence
            .values()
            .flat_map(|e| e.prepare_qc.iter().chain(e.accept_qc.iter()))
    }

    pub fn add_shard_group(&mut self, shard_group: ShardGroup) -> &mut ShardGroupEvidence {
        if !self.evidence.contains_key(&shard_group) {
            // We cannot use entry() because we want to insert sorted
            self.evidence.insert_sorted(shard_group, ShardGroupEvidence::default());
        }
        self.evidence.get_mut(&shard_group).expect("added above")
    }

    pub fn shard_groups_iter(&self) -> impl Iterator<Item = &ShardGroup> {
        self.evidence.keys()
    }

    pub fn input_shard_groups_iter(&self) -> impl Iterator<Item = &ShardGroup> {
        self.evidence
            .iter()
            .filter_map(|(sg, e)| if e.inputs.is_empty() { None } else { Some(sg) })
    }

    pub fn num_shard_groups(&self) -> usize {
        self.evidence.len()
    }

    /// Returns the portion of a transaction's exhaust burn attributed to `shard_group`, or `None` if the shard group
    /// is not part of this evidence.
    ///
    /// The whole-transaction burn is divided evenly among the involved shard groups, with one extra unit going to each
    /// of the first `exhaust_burn % num_shard_groups` groups in `ShardGroup` order, so the portions sum to exactly
    /// `exhaust_burn`. Each shard group accumulates only its portion into its block header burn total, so summing the
    /// accumulated burn across all shard groups counts each transaction's burn exactly once.
    ///
    /// CONSENSUS RULE: every involved shard group must attribute portions identically. Groups are ranked by
    /// `ShardGroup` order, relying on the sorted-key invariant maintained by all local constructors
    /// (`insert_sorted`/`sort_keys`). Wire-decoded evidence preserves the sender's key order and passes
    /// order-independent equality checks, so callers must pass locally-constructed evidence (e.g. the local pool
    /// record's), never evidence decoded from a received proposal.
    pub fn exhaust_burn_portion(&self, exhaust_burn: u64, shard_group: ShardGroup) -> Option<u64> {
        debug_assert!(self.evidence.keys().is_sorted(), "evidence keys must be sorted");
        let index = self.evidence.keys().position(|sg| *sg == shard_group)? as u64;
        let num_groups = self.evidence.len() as u64;
        let base_portion = exhaust_burn / num_groups;
        let remainder = exhaust_burn % num_groups;
        // The first `remainder` groups in ShardGroup order each take one extra unit of the burn
        let extra = if index < remainder { 1 } else { 0 };
        Some(base_portion + extra)
    }

    /// Add or update shard groups, substates and locks into Evidence. Existing prepare/accept QC IDs are not changed.
    pub fn merge(&mut self, other: &Evidence) -> &mut Self {
        for (sg, evidence) in other.iter() {
            let evidence_mut = self.evidence.entry(*sg).or_default();
            let inputs_mut = &mut evidence_mut.inputs;

            for (substate_id, other_evidence) in evidence.inputs.iter().map(|(id, lock)| (id.clone(), *lock)) {
                if let Some(e_mut) = inputs_mut.get_mut(&substate_id) {
                    match other_evidence {
                        Some(e) => match e_mut {
                            Some(e_mut) => {
                                e_mut.is_write = e.is_write;
                                e_mut.version = e.version;
                            },
                            None => {
                                *e_mut = Some(e);
                            },
                        },
                        None => continue,
                    }
                } else {
                    inputs_mut.insert(substate_id, other_evidence);
                }
            }
            evidence_mut
                .outputs
                .extend(evidence.outputs.iter().map(|(id, version)| (id.clone(), *version)));
            evidence_mut.sort_substates();
        }
        self.evidence.sort_keys();
        self
    }

    pub fn eq_pledges(&self, other: &Evidence) -> bool {
        if self.len() != other.len() {
            debug!(
                target: LOG_TARGET,
                "Evidence length mismatch: self={}, other={}",
                self.len(),
                other.len()
            );
            return false;
        }

        for (sg, evidence) in self.iter() {
            if let Some(other_evidence) = other.get(sg) {
                if evidence.inputs() != other_evidence.inputs() {
                    debug!(
                        target: LOG_TARGET,
                        "Inputs mismatch for shard group {}: self={:?}, other={:?}",
                        sg,
                        evidence.inputs(),
                        other_evidence.inputs()
                    );
                    return false;
                }
                if evidence.outputs() != other_evidence.outputs() {
                    debug!(
                        target: LOG_TARGET,
                        "Outputs mismatch for shard group {}: self={:?}, other={:?}",
                        sg,
                        evidence.outputs(),
                        other_evidence.outputs()
                    );
                    return false;
                }
            } else {
                debug!(target: LOG_TARGET, "Missing shard group evidence for {}", sg);
                return false;
            }
        }
        true
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
        if f.alternate() {
            if self.is_empty() {
                write!(f, "{{EMPTY}}")?;
                return Ok(());
            }

            for (i, (shard_group, shard_evidence)) in self.evidence.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                match (
                    !shard_evidence.inputs().is_empty(),
                    shard_evidence.is_prepare_justified(),
                    shard_evidence.is_accept_justified(),
                ) {
                    (true, true, true) => write!(f, "{}: ACCEPTED", shard_group)?,
                    (true, false, true) => write!(f, "{}: ACCEPTED (not prepared)", shard_group)?,
                    (true, true, false) => write!(f, "{}: PREPARED (need accept)", shard_group)?,
                    // Output only
                    (false, _, true) => write!(f, "{}: ACCEPTED (output-only)", shard_group)?,
                    (false, true, false) => write!(f, "{}: PREPARED (output-only, need accept)", shard_group)?,
                    _ => write!(f, "{}: NO EVIDENCE", shard_group)?,
                }
            }
        } else {
            if self.is_empty() {
                write!(f, "{{EMPTY}}")?;
                return Ok(());
            }
            write!(f, "{{")?;
            for (i, (substate_address, shard_evidence)) in self.evidence.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}: {}", substate_address, shard_evidence)?;
            }
            write!(f, "}}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ShardGroupEvidence {
    #[borsh(serialize_with = "indexmap_borsh::serialize")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, any>"))]
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::indexmap_codec")]
    inputs: IndexMap<SubstateId, Option<EvidenceInputLockData>>,
    #[borsh(serialize_with = "indexmap_borsh::serialize")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, number>"))]
    #[n(1)]
    #[cbor(with = "tari_bor::adapters::indexmap_codec")]
    outputs: IndexMap<SubstateId, u64>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[n(2)]
    prepare_qc: Option<PcId>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[n(3)]
    accept_qc: Option<PcId>,
}

impl ShardGroupEvidence {
    pub fn insert_from_lock_intent<T: LockIntent>(&mut self, lock: T) -> &mut Self {
        self.insert(lock.substate_id().clone(), lock.version_to_lock(), lock.lock_type())
    }

    pub fn insert(&mut self, substate_id: SubstateId, version_to_lock: u64, lock_type: SubstateLockType) -> &mut Self {
        if lock_type.is_input() {
            self.inputs.insert_sorted(
                substate_id,
                Some(EvidenceInputLockData {
                    is_write: lock_type.is_write(),
                    version: version_to_lock,
                }),
            );
        } else {
            self.outputs.insert_sorted(substate_id, version_to_lock);
        }
        self
    }

    pub fn insert_unpledged_input(&mut self, substate_id: SubstateId) -> &mut Self {
        self.inputs.insert_sorted(substate_id, None);
        self
    }

    pub fn insert_output(&mut self, substate_id: SubstateId, version: u64) -> &mut Self {
        self.outputs.insert_sorted(substate_id, version);
        self
    }

    pub fn is_prepare_justified(&self) -> bool {
        self.prepare_qc.is_some()
    }

    pub fn is_accept_justified(&self) -> bool {
        self.accept_qc.is_some()
    }

    pub fn is_all_inputs_pledged(&self) -> bool {
        self.inputs.iter().all(|(_, lock)| lock.is_some())
    }

    pub fn inputs(&self) -> &IndexMap<SubstateId, Option<EvidenceInputLockData>> {
        &self.inputs
    }

    pub fn all_pledged_inputs_iter(&self) -> impl Iterator<Item = (&SubstateId, &EvidenceInputLockData)> {
        self.inputs.iter().filter_map(|(id, ev)| Some((id, ev.as_ref()?)))
    }

    pub fn input_lock_intents(&self) -> impl Iterator<Item = RequireLockIntentRef<'_>> + '_ {
        self.inputs.iter().filter_map(|(substate_id, e)| {
            e.as_ref()
                .map(|lock| RequireLockIntentRef::new(substate_id, lock.version, lock.as_lock_type()))
        })
    }

    pub fn outputs(&self) -> &IndexMap<SubstateId, u64> {
        &self.outputs
    }

    pub fn output_pledge_iter(&self) -> impl Iterator<Item = SubstatePledge> + '_ {
        self.outputs
            .iter()
            .map(|(substate_id, version)| SubstatePledge::Output {
                substate_id: VersionedSubstateId::new(substate_id.clone(), *version),
            })
    }

    pub fn abort(&mut self) -> &mut Self {
        for (_, input) in &mut self.inputs {
            *input = None;
        }

        self.outputs = IndexMap::new();
        self
    }

    fn sort_substates(&mut self) {
        self.inputs.sort_keys();
        self.outputs.sort_keys();
    }

    pub fn contains_pledge(&self, substate_id: &SubstateId, version: u64, is_input: bool) -> bool {
        if is_input {
            return self
                .inputs
                .get(substate_id)
                .is_some_and(|e| e.as_ref().is_some_and(|e| e.version == version));
        }

        self.outputs.get(substate_id).is_some_and(|v| *v == version)
    }

    pub fn update(&mut self, other: &ShardGroupEvidence) -> &mut Self {
        for (substate_id, lock) in &other.inputs {
            if let Some(e) = lock {
                if let Some(ev_mut) = self.inputs.get_mut(substate_id) {
                    *ev_mut = Some(*e);
                } else {
                    self.inputs.insert_sorted(substate_id.clone(), Some(*e));
                }
            } else if !self.inputs.contains_key(substate_id) {
                self.inputs.insert_sorted(substate_id.clone(), None);
            } else {
                // Do nothing
            }
        }
        for (substate_id, version) in &other.outputs {
            if let Some(v) = self.outputs.get_mut(substate_id) {
                *v = *version;
            } else {
                self.outputs.insert_sorted(substate_id.clone(), *version);
            }
        }
        self
    }

    pub fn set_prepare_qc(&mut self, qc_id: PcId) -> &mut Self {
        debug!(
            target: LOG_TARGET,
            "set_prepare_qc: QC[{qc_id}]",
        );
        self.prepare_qc = Some(qc_id);
        self
    }

    pub fn prepare_qc(&self) -> Option<&PcId> {
        self.prepare_qc.as_ref()
    }

    pub fn set_accept_qc(&mut self, qc_id: PcId) -> &mut Self {
        debug!(
            target: LOG_TARGET,
            "set_accept_qc: QC[{qc_id}]",
        );
        self.accept_qc = Some(qc_id);
        self
    }

    pub fn accept_qc(&self) -> Option<&PcId> {
        self.accept_qc.as_ref()
    }
}

impl Display for ShardGroupEvidence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "inputs[")?;
        for (i, (substate_id, lock)) in self.inputs.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", substate_id, lock.display())?;
        }
        write!(f, "],")?;
        write!(f, "outputs[")?;
        for (i, (substate_id, version)) in self.outputs.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", substate_id, version)?;
        }
        write!(f, "]")?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EvidenceInputLockData {
    #[n(0)]
    pub is_write: bool,
    #[n(1)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
}

impl EvidenceInputLockData {
    pub fn as_lock_type(&self) -> SubstateLockType {
        if self.is_write {
            SubstateLockType::Write
        } else {
            SubstateLockType::Read
        }
    }
}

impl Display for EvidenceInputLockData {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let rw = if self.is_write { "Write" } else { "Read" };
        write!(f, "v{} {}", self.version, rw)
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::SubstateLockType;
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;
    use crate::consensus_models::SubstateRequirementLockIntent;

    fn seed_substate_id(seed: u8) -> SubstateId {
        SubstateId::Component(ComponentAddress::from_array([seed; ObjectKey::LENGTH]))
    }

    fn seed_lock_intent(seed: u8, ty: SubstateLockType) -> SubstateRequirementLockIntent {
        SubstateRequirementLockIntent::new(seed_substate_id(seed), 0, ty)
    }

    mod exhaust_burn_portion {
        use super::*;

        fn evidence_with_groups(groups: &[ShardGroup]) -> Evidence {
            let mut evidence = Evidence::empty();
            for (i, sg) in groups.iter().enumerate() {
                evidence
                    .add_shard_group(*sg)
                    .insert_from_lock_intent(seed_lock_intent(i as u8 + 1, SubstateLockType::Write));
            }
            evidence
        }

        #[test]
        fn it_assigns_the_remainder_to_the_first_groups_in_shard_group_order() {
            let groups = [ShardGroup::new(0, 1), ShardGroup::new(2, 3), ShardGroup::new(4, 5)];
            let evidence = evidence_with_groups(&groups);

            assert_eq!(evidence.exhaust_burn_portion(19, groups[0]), Some(7));
            assert_eq!(evidence.exhaust_burn_portion(19, groups[1]), Some(6));
            assert_eq!(evidence.exhaust_burn_portion(19, groups[2]), Some(6));

            // Burn smaller than the number of groups
            assert_eq!(evidence.exhaust_burn_portion(2, groups[0]), Some(1));
            assert_eq!(evidence.exhaust_burn_portion(2, groups[1]), Some(1));
            assert_eq!(evidence.exhaust_burn_portion(2, groups[2]), Some(0));

            assert_eq!(evidence.exhaust_burn_portion(0, groups[0]), Some(0));
        }

        #[test]
        fn it_sums_to_exactly_the_whole_burn() {
            let groups = [
                ShardGroup::new(0, 1),
                ShardGroup::new(2, 3),
                ShardGroup::new(4, 5),
                ShardGroup::new(6, 7),
            ];
            for num_groups in 1..=groups.len() {
                let evidence = evidence_with_groups(&groups[..num_groups]);
                for burn in [0u64, 1, 3, 5, 19, 20, 100, 1_000_003] {
                    let total = groups[..num_groups]
                        .iter()
                        .map(|sg| evidence.exhaust_burn_portion(burn, *sg).unwrap())
                        .sum::<u64>();
                    assert_eq!(
                        total, burn,
                        "burn {burn} was created or lost across {num_groups} group(s)"
                    );
                }
            }
        }

        #[test]
        fn it_returns_none_for_an_uninvolved_shard_group() {
            let evidence = evidence_with_groups(&[ShardGroup::new(0, 1)]);
            assert_eq!(evidence.exhaust_burn_portion(10, ShardGroup::new(2, 3)), None);
        }

        #[test]
        fn it_does_not_depend_on_insertion_order() {
            let sg1 = ShardGroup::new(0, 1);
            let sg2 = ShardGroup::new(2, 3);
            let sg3 = ShardGroup::new(4, 5);

            let mut evidence = Evidence::empty();
            evidence.add_shard_group(sg3);
            evidence.add_shard_group(sg1);
            evidence.add_shard_group(sg2);

            assert_eq!(evidence.exhaust_burn_portion(19, sg1), Some(7));
            assert_eq!(evidence.exhaust_burn_portion(19, sg2), Some(6));
            assert_eq!(evidence.exhaust_burn_portion(19, sg3), Some(6));
        }
    }

    #[test]
    fn it_merges_two_evidences_together() {
        let sg1 = ShardGroup::new(0, 1);
        let sg2 = ShardGroup::new(2, 3);
        let sg3 = ShardGroup::new(4, 5);

        let mut evidence1 = Evidence::empty();
        evidence1
            .add_shard_group(sg1)
            .insert_from_lock_intent(seed_lock_intent(1, SubstateLockType::Write));
        evidence1
            .add_shard_group(sg1)
            .insert_from_lock_intent(seed_lock_intent(2, SubstateLockType::Read));

        let mut evidence2 = Evidence::empty();
        evidence2
            .add_shard_group(sg1)
            .insert_from_lock_intent(seed_lock_intent(2, SubstateLockType::Write));
        evidence2
            .add_shard_group(sg1)
            .insert_from_lock_intent(seed_lock_intent(2, SubstateLockType::Output));
        evidence2
            .add_shard_group(sg2)
            .insert_from_lock_intent(seed_lock_intent(3, SubstateLockType::Output));
        evidence2
            .add_shard_group(sg3)
            .insert_from_lock_intent(seed_lock_intent(4, SubstateLockType::Output));

        evidence1.merge(&evidence2);

        assert_eq!(evidence1.len(), 3);
        assert!(
            evidence1
                .get(&sg1)
                .unwrap()
                .inputs
                .get(&seed_substate_id(1))
                .unwrap()
                .unwrap()
                .is_write,
        );
        assert!(
            evidence1
                .get(&sg1)
                .unwrap()
                .inputs
                .get(&seed_substate_id(2))
                .unwrap()
                .unwrap()
                .is_write,
        );
        assert_eq!(
            evidence1.get(&sg1).unwrap().outputs.get(&seed_substate_id(2)),
            Some(&0u64)
        );
        assert_eq!(
            evidence1.get(&sg1).unwrap().outputs.get(&seed_substate_id(2)),
            Some(&0u64)
        );
    }
}
