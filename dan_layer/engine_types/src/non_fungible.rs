//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tari_bor::{decode_exact, BorError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonFungibleContainer(Option<NonFungible>);

impl NonFungibleContainer {
    pub fn no_data() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    pub fn new(data: Vec<u8>, mutable_data: Vec<u8>) -> Self {
        Self(Some(NonFungible::new(data, mutable_data)))
    }

    pub fn contents_mut(&mut self) -> Option<&mut NonFungible> {
        self.0.as_mut()
    }

    pub fn contents(&self) -> Option<&NonFungible> {
        self.0.as_ref()
    }

    pub fn is_burnt(&self) -> bool {
        self.0.is_none()
    }

    pub fn burn(&mut self) {
        self.0 = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonFungible {
    data: Vec<u8>,
    mutable_data: Vec<u8>,
}

impl NonFungible {
    pub fn new(data: Vec<u8>, mutable_data: Vec<u8>) -> Self {
        Self { data, mutable_data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn mutable_data(&self) -> &[u8] {
        &self.mutable_data
    }

    pub fn decode_mutable_data<T: DeserializeOwned>(&self) -> Result<T, BorError> {
        decode_exact(&self.mutable_data)
    }

    pub fn set_mutable_data(&mut self, mutable_data: Vec<u8>) {
        self.mutable_data = mutable_data;
    }
}
