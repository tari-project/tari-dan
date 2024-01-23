//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use tari_engine_types::virtual_substate::{VirtualSubstate, VirtualSubstateId};

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
