//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::any::type_name;

use serde::Serialize;
use tari_dan_wallet_sdk::storage::WalletStorageError;

pub fn serialize_json<T: Serialize + ?Sized>(t: &T) -> Result<String, WalletStorageError> {
    serde_json::to_string(t).map_err(|e| WalletStorageError::EncodingError {
        operation: "serialize_json",
        item: type_name::<T>(),
        details: e.to_string(),
    })
}

pub fn deserialize_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, WalletStorageError> {
    serde_json::from_str(s).map_err(|e| WalletStorageError::DecodingError {
        operation: "deserialize_json",
        item: type_name::<T>(),
        details: e.to_string(),
    })
}
