//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::BTreeSet;

use tari_bor::{borsh, Decode, Encode};
use tari_template_lib::{
    models::{Amount, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
};

use crate::resource_container::{ResourceContainer, ResourceError};

#[derive(Debug, Clone, Encode, Decode)]
pub struct Bucket {
    resource: ResourceContainer,
}

impl Bucket {
    pub fn new(resource: ResourceContainer) -> Self {
        Self { resource }
    }

    pub fn amount(&self) -> Amount {
        self.resource.amount()
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        self.resource.resource_address()
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource.resource_type()
    }

    pub fn into_resource(self) -> ResourceContainer {
        self.resource
    }

    pub fn into_non_fungible_ids(self) -> Option<BTreeSet<NonFungibleId>> {
        self.resource.into_non_fungible_ids()
    }

    pub fn take(&mut self, amount: Amount) -> Result<ResourceContainer, ResourceError> {
        self.resource.withdraw(amount)
    }
}
