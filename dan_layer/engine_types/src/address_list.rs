//   Copyright 2023. The Tari Project
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

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, Decode, Encode};
use tari_template_lib::models::AddressListId;

use crate::substate::SubstateAddress;

/// Placeholder for empty address lists, so they can have an address in the network
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, PartialEq)]
pub struct AddressList {
    id: AddressListId,
}

impl AddressList {
    pub fn new(id: AddressListId) -> Self {
        Self { id }
    }
}

/// Holds a reference to another substate
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, PartialEq)]
pub struct AddressListItem {
    list_id: AddressListId,
    index: u64,
    referenced_address: SubstateAddress,
}

impl AddressListItem {
    pub fn new(list_id: AddressListId, index: u64, referenced_address: SubstateAddress) -> Self {
        Self {
            list_id,
            index,
            referenced_address,
        }
    }

    pub fn list_id(&self) -> &AddressListId {
        &self.list_id
    }

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn referenced_address(&self) -> &SubstateAddress {
        &self.referenced_address
    }
}
