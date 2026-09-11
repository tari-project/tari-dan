//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::mem;

use indexmap::{IndexMap, IndexSet};
use tari_bor::Value;
use tari_engine_types::indexed_value::{IndexedValue, IndexedValueError};
use tari_ootle_transaction::args::{WorkspaceId, WorkspaceOffsetId};
use tari_template_lib::models::ProofId;

use crate::runtime::RuntimeError;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Indexed value error: {0}")]
    IndexedValueError(#[from] IndexedValueError),
    #[error("Workspace ID {0} already exists in the workspace")]
    WorkspaceIdAlreadyExists(WorkspaceId),
}

/// NOTE: the collections here must be insertion-ordered rather than hashed. `drain_all_proofs` drives the order in
/// which proofs are dropped, and dropping a vault-backed proof can append that vault to the substates the
/// transaction persists — whose order becomes `DiffSummary::upped` in the `TransactionReceipt`, and is hashed.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    items: IndexMap<WorkspaceId, IndexedValue>,
    proofs: IndexSet<ProofId>,
}

impl Workspace {
    pub fn get(&self, offset_id: WorkspaceOffsetId) -> Result<Option<&Value>, RuntimeError> {
        let Some(value) = self.items.get(&offset_id.id()) else {
            // if the value is not found, we return None
            return Ok(None);
        };
        let Some(offset) = offset_id.offset() else {
            // if the offset is None, we return the whole value
            return Ok(Some(value.value()));
        };
        match value.value() {
            Value::Array(items) => Ok(items.get(offset)),
            Value::Map(items) => Ok(items.get(offset).map(|(_, v)| v)),
            // Unsupported value types
            _ => Ok(None),
        }
    }

    pub fn insert(&mut self, id: WorkspaceId, value: IndexedValue) -> Result<(), WorkspaceError> {
        if !value.proof_ids().is_empty() {
            self.proofs.extend(value.proof_ids().iter().copied());
        }
        if self.items.contains_key(&id) {
            return Err(WorkspaceError::WorkspaceIdAlreadyExists(id));
        }

        self.items.insert(id, value);
        Ok(())
    }

    pub fn drain_all_proofs(&mut self) -> IndexSet<ProofId> {
        mem::take(&mut self.proofs)
    }

    pub fn clear_items(&mut self) {
        self.items.clear();
    }

    pub fn all_ids_iter(&self) -> impl Iterator<Item = WorkspaceId> + '_ {
        self.items.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use tari_engine_types::indexed_value::IndexedValue;

    use super::*;

    #[test]
    fn tuples() {
        // create the tuple value
        let tuple = ("Foo", 32);
        let encoded_tuple = IndexedValue::from_type(&tuple).unwrap();

        // add the tuple to the workspace
        let mut workspace = Workspace::default();
        workspace.insert(1, encoded_tuple.clone()).unwrap();

        // the tuple itself can be retrieved
        let value = workspace.get(WorkspaceOffsetId::new(1)).unwrap().unwrap();
        assert_eq!(value, encoded_tuple.value());

        // each tuple item can be addresed individually
        // item 0
        let expected = IndexedValue::from_type(&tuple.0).unwrap();
        let value = workspace
            .get(WorkspaceOffsetId::new(1).with_offset(0))
            .unwrap()
            .unwrap();
        assert_eq!(value, expected.value());
        // item 1
        let expected = IndexedValue::from_type(&tuple.1).unwrap();
        let value = workspace
            .get(WorkspaceOffsetId::new(1).with_offset(1))
            .unwrap()
            .unwrap();
        assert_eq!(value, expected.value());
    }
}
