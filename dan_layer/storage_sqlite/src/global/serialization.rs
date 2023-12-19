//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::any::type_name;

use serde::Serialize;

use crate::error::SqliteStorageError;

pub fn serialize_json<T: Serialize + ?Sized>(t: &T) -> Result<String, SqliteStorageError> {
    serde_json::to_string_pretty(t).map_err(|e| SqliteStorageError::EncodingError {
        operation: "serialize_json",
        item: type_name::<T>(),
        details: e.to_string(),
    })
}

pub fn deserialize_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, SqliteStorageError> {
    serde_json::from_str(s).map_err(|e| SqliteStorageError::DecodingError {
        operation: "deserialize_json",
        item: type_name::<T>(),
        details: e.to_string(),
    })
}
