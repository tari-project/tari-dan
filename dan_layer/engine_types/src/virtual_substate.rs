//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;

use crate::fee_claim::FeeClaim;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VirtualSubstateId {
    CurrentEpoch,
    UnclaimedValidatorFee { epoch: u64, address: PublicKey },
}

impl Display for VirtualSubstateId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            VirtualSubstateId::CurrentEpoch => write!(f, "Virtual(CurrentEpoch)"),
            VirtualSubstateId::UnclaimedValidatorFee { epoch, address } => {
                write!(
                    f,
                    "Virtual(UnclaimedValidatorFee(epoch = {}, address = {:.7}))",
                    epoch, address
                )
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VirtualSubstate {
    CurrentEpoch(u64),
    UnclaimedValidatorFee(FeeClaim),
}

// Developer note: this struct has two non-functional purposes:
// 1. so that we do not have to type out the HashMap type in many places, and
// 2. so that the clippy::mutable_key_type annotation is not needed in many places

/// Virtual substate collection
#[derive(Debug, Clone, Default)]
pub struct VirtualSubstates(HashMap<VirtualSubstateId, VirtualSubstate>);

impl VirtualSubstates {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashMap::with_capacity(capacity))
    }
}

impl Deref for VirtualSubstates {
    type Target = HashMap<VirtualSubstateId, VirtualSubstate>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VirtualSubstates {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for VirtualSubstates {
    type IntoIter = <HashMap<VirtualSubstateId, VirtualSubstate> as IntoIterator>::IntoIter;
    type Item = <HashMap<VirtualSubstateId, VirtualSubstate> as IntoIterator>::Item;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(VirtualSubstateId, VirtualSubstate)> for VirtualSubstates {
    fn from_iter<T: IntoIterator<Item = (VirtualSubstateId, VirtualSubstate)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}
