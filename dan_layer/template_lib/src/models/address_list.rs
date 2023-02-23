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
use tari_template_abi::{
    call_engine,
    rust::{
        fmt,
        fmt::{Display, Formatter},
    },
    EngineOp,
};

use super::Address;
use crate::{
    args::{AddressListAction, AddressListInvokeArg, InvokeResult},
    hash::HashParseError,
    Hash,
};

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct AddressListItemAddress {
    list_id: AddressListId,
    index: u64,
}

impl AddressListItemAddress {
    pub fn new(list_id: AddressListId, index: u64) -> Self {
        Self { list_id, index }
    }

    pub fn list_id(&self) -> &AddressListId {
        &self.list_id
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}

impl Display for AddressListItemAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} index_{}", self.list_id, self.index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AddressListId(Hash);

impl AddressListId {
    pub fn new(address: Hash) -> Self {
        Self(address)
    }

    pub fn hash(&self) -> &Hash {
        &self.0
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self::new(hash))
    }
}

impl From<Hash> for AddressListId {
    fn from(address: Hash) -> Self {
        Self::new(address)
    }
}

impl Display for AddressListId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "addresslist_{}", self.0)
    }
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct AddressList {
    id: AddressListId,
}

impl AddressList {
    pub fn new() -> Self {
        let resp: InvokeResult = call_engine(EngineOp::AddressListInvoke, &AddressListInvokeArg {
            list_id: None,
            action: AddressListAction::Create,
            args: args![],
        });

        Self {
            id: resp.decode().unwrap(),
        }
    }

    // TODO: ideally the caller should not need to pass the "index" parameter, it should be automatically calculated by
    // the network       but we don't have that functionality yet implemented
    pub fn push(&mut self, index: u64, address: Address) {
        let result: InvokeResult = call_engine(EngineOp::AddressListInvoke, &AddressListInvokeArg {
            list_id: Some(self.id),
            action: AddressListAction::Push,
            args: invoke_args![index, address],
        });

        result.decode::<()>().expect("push failed");
    }
}

impl Default for AddressList {
    fn default() -> Self {
        Self::new()
    }
}
