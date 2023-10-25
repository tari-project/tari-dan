//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    mem,
};

use tari_engine_types::indexed_value::{IndexedValue, IndexedValueError};
use tari_template_lib::models::ProofId;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    // #[error("Value decoding error: {0}")]
    // ValueDecodingError(#[from] BorError),
    #[error("Indexed value error: {0}")]
    IndexedValueError(#[from] IndexedValueError),
}

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    variables: HashMap<Vec<u8>, IndexedValue>,
    proofs: HashSet<ProofId>,
}

impl Workspace {
    pub fn get(&self, key: &[u8]) -> Option<&IndexedValue> {
        self.variables.get(key)
    }

    pub fn insert(&mut self, key: Vec<u8>, value: IndexedValue) -> Result<(), WorkspaceError> {
        // if the value is an array then we need to add entries for all items
        // TODO: support for structs
        if let tari_bor::Value::Array(items) = value.value() {
            let key_str = String::from_utf8_lossy(&key);

            for (i, item) in items.clone().into_iter().enumerate() {
                let item_value = IndexedValue::from_value(item)?;

                // we do not have a way to differentiate tuples from arrays, so we support both
                let item_tuple_key = format!("{}.{}", key_str, i);
                let item_array_key = format!("{}[{}]", key_str, i);
                self.insert_internal(item_tuple_key.into(), item_value.clone());
                self.insert_internal(item_array_key.into(), item_value);
            }
        }

        self.insert_internal(key, value);

        Ok(())
    }

    fn insert_internal(&mut self, key: Vec<u8>, value: IndexedValue) {
        if !value.proof_ids().is_empty() {
            self.proofs.extend(value.proof_ids().iter().copied());
        }
        self.variables.insert(key, value);
    }

    pub fn drain_all_proofs(&mut self) -> HashSet<ProofId> {
        mem::take(&mut self.proofs)
    }
}

#[cfg(test)]
mod tests {
    use tari_engine_types::indexed_value::IndexedValue;
    use tari_utilities::ByteArray;

    use super::Workspace;

    #[test]
    fn tuples() {
        // create the tuple value
        let tuple = ("Foo", 32);
        let encoded_tuple = IndexedValue::from_type(&tuple).unwrap();

        // add the tuple to the workspace
        let mut workspace = Workspace::default();
        workspace.insert(b"tuple".to_vec(), encoded_tuple.clone()).unwrap();

        // the tuple itself can be retrieved
        let value = workspace.get(b"tuple").unwrap();
        assert_eq!(*value, encoded_tuple);

        // each tuple item can be addresed individually
        // item 0
        let expected = IndexedValue::from_type(&tuple.0).unwrap();
        let value = workspace.get(b"tuple.0").unwrap();
        assert_eq!(*value, expected);
        // item 1
        let expected = IndexedValue::from_type(&tuple.1).unwrap();
        let value = workspace.get(b"tuple.1").unwrap();
        assert_eq!(*value, expected);
    }
}
