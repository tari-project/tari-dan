// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use tari_template_abi::{Decode, Encode};
use tari_template_lib::models::{Amount, ResourceAddress};

use crate::models::resource::Resource;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Bucket {
    resource: Resource,
}

impl Bucket {
    pub fn new(resource: Resource) -> Self {
        Self { resource }
    }

    pub fn amount(&self) -> Amount {
        self.resource.amount()
    }

    pub fn resource_address(&self) -> ResourceAddress {
        self.resource.address()
    }

    pub fn resource(&self) -> &Resource {
        &self.resource
    }

    pub fn into_resource(self) -> Resource {
        self.resource
    }
}
