//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_bor::{decode, encode, BorError, Value};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("Value decoding error: {0}")]
    ValueDecodingError(#[from] BorError),
}

#[derive(Debug, Clone, Default)]
pub(super) struct Workspace {
    pub variables: HashMap<Vec<u8>, Vec<u8>>,
}

impl Workspace {
    pub fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.variables.get(key)
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), WorkspaceError> {
        self.variables.insert(key.clone(), value.clone());

        // if the value is an array then we need to add entries for all items
        let value: tari_bor::Value = decode(&value)?;
        // TODO: support for structs
        if let Value::Array(items) = value {
            let key_str = String::from_utf8_lossy(&key);

            for (i, item) in items.iter().enumerate() {
                let item_value = encode(item)?;

                // we do not have a way to differentiate tuples from arrays, so we support both
                let item_tuple_key = format!("{}.{}", key_str, i);
                let item_array_key = format!("{}[{}]", key_str, i);
                self.variables.insert(item_tuple_key.into(), item_value.clone());
                self.variables.insert(item_array_key.into(), item_value);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_bor::encode;
    use tari_utilities::ByteArray;

    use super::Workspace;

    #[test]
    fn tuples() {
        // create the tuple value
        let tuple = ("Foo", 32);
        let encoded_tuple = encode(&tuple).unwrap();

        // add the tuple to the workspace
        let mut workspace = Workspace::default();
        workspace.insert(b"tuple".to_vec(), encoded_tuple.clone()).unwrap();

        // the tuple itself can be retrieved
        let value = workspace.get(b"tuple").unwrap();
        assert_eq!(*value, encoded_tuple);

        // each tuple item can be addresed individually
        // item 0
        let expected = encode(&tuple.0).unwrap();
        let value = workspace.get(b"tuple.0").unwrap();
        assert_eq!(*value, expected);
        // item 1
        let expected = encode(&tuple.1).unwrap();
        let value = workspace.get(b"tuple.1").unwrap();
        assert_eq!(*value, expected);
    }
}
