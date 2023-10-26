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

use tari_template_lib::{
    models::{Amount, BucketId, NonFungibleId, ResourceAddress, VaultId},
    prelude::ResourceType,
};

use crate::resource_container::ResourceContainer;

#[derive(Debug, Clone)]
pub struct Proof {
    locked: LockedResource,
}

impl Proof {
    pub fn new(locked: LockedResource) -> Self {
        Self { locked }
    }

    pub fn amount(&self) -> Amount {
        self.locked.amount()
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        self.locked.resource_address()
    }

    pub fn non_fungible_token_ids(&self) -> &BTreeSet<NonFungibleId> {
        self.locked.non_fungible_token_ids()
    }

    pub fn resource_type(&self) -> ResourceType {
        self.locked.resource_type()
    }

    pub fn container(&self) -> &ContainerRef {
        &self.locked.container
    }

    pub fn into_resource_container(self) -> ResourceContainer {
        self.locked.into_resource_container()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ContainerRef {
    Bucket(BucketId),
    Vault(VaultId),
}

#[derive(Debug, Clone)]
pub struct LockedResource {
    container: ContainerRef,
    locked: ResourceContainer,
}

impl LockedResource {
    pub fn new(container: ContainerRef, locked: ResourceContainer) -> Self {
        Self { container, locked }
    }

    pub fn amount(&self) -> Amount {
        self.locked.amount()
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        self.locked.resource_address()
    }

    pub fn non_fungible_token_ids(&self) -> &BTreeSet<NonFungibleId> {
        self.locked.non_fungible_token_ids()
    }

    pub fn resource_type(&self) -> ResourceType {
        self.locked.resource_type()
    }

    pub fn container(&self) -> &ContainerRef {
        &self.container
    }

    pub fn into_resource_container(self) -> ResourceContainer {
        self.locked
    }
}
