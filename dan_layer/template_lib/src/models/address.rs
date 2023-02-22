//  Copyright 2023. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_bor::{borsh, Decode, Encode};

use super::{AddressListId, ComponentAddress, NonFungibleAddress, ResourceAddress, VaultId};

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Address {
    Component(ComponentAddress),
    Resource(ResourceAddress),
    Vault(VaultId),
    NonFungible(NonFungibleAddress),
    AddressList(AddressListId),
    // There is no variant for AddressListItem as the concept of items is not exposed in templates
}

impl From<ComponentAddress> for Address {
    fn from(address: ComponentAddress) -> Self {
        Self::Component(address)
    }
}

impl From<ResourceAddress> for Address {
    fn from(address: ResourceAddress) -> Self {
        Self::Resource(address)
    }
}

impl From<VaultId> for Address {
    fn from(address: VaultId) -> Self {
        Self::Vault(address)
    }
}

impl From<NonFungibleAddress> for Address {
    fn from(address: NonFungibleAddress) -> Self {
        Self::NonFungible(address)
    }
}

impl From<AddressListId> for Address {
    fn from(address: AddressListId) -> Self {
        Self::AddressList(address)
    }
}
