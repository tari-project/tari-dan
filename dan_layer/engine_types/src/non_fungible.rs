//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tari_bor::BorError;
use tari_template_lib::prelude::Metadata;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct NonFungibleContainer(Option<NonFungible>);

impl NonFungibleContainer {
    pub fn no_data() -> Self {
        Self::new(tari_bor::Value::Null, tari_bor::Value::Null)
    }

    pub fn new(data: tari_bor::Value, mutable_data: tari_bor::Value) -> Self {
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
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct NonFungible {
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    data: tari_bor::Value,
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    mutable_data: tari_bor::Value,
}

impl NonFungible {
    pub fn new(data: tari_bor::Value, mutable_data: tari_bor::Value) -> Self {
        Self { data, mutable_data }
    }

    pub fn data(&self) -> &tari_bor::Value {
        &self.data
    }

    pub fn mutable_data(&self) -> &tari_bor::Value {
        &self.mutable_data
    }

    pub fn decode_mutable_data<T: DeserializeOwned>(&self) -> Result<T, BorError> {
        tari_bor::from_value(&self.mutable_data)
    }

    pub fn decode_data(&self) -> Result<Metadata, BorError> {
        if self.data.is_null() {
            return Ok(Metadata::default());
        }
        tari_bor::from_value(&self.data)
    }

    pub fn set_mutable_data(&mut self, mutable_data: tari_bor::Value) {
        self.mutable_data = mutable_data;
    }
}
