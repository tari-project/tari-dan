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

use std::{error::Error, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_template_abi::rust::{fmt, fmt::Display};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::ResourceAddress;

/// The unique identifier of a non-fungible index in the Tari network
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct NonFungibleIndexAddress {
    resource_address: ResourceAddress,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    index: u64,
}

impl NonFungibleIndexAddress {
    pub const fn new(resource_address: ResourceAddress, index: u64) -> Self {
        Self {
            resource_address,
            index,
        }
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        &self.resource_address
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}

impl Display for NonFungibleIndexAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nftindex_")?;
        for byte in self.resource_address.as_bytes() {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, "_{}", self.index)
    }
}

impl FromStr for NonFungibleIndexAddress {
    type Err = NonFungibleIndexAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // nftindex_{resource_id}_{index}
        let s = s.strip_prefix("nftindex_").unwrap_or(s);
        let (resource, index) = s.split_once('_').ok_or(NonFungibleIndexAddressParseError)?;
        let resource_address = resource.parse().map_err(|_| NonFungibleIndexAddressParseError)?;
        let index = index.parse().map_err(|_| NonFungibleIndexAddressParseError)?;
        Ok(Self::new(resource_address, index))
    }
}

#[derive(Debug)]
pub struct NonFungibleIndexAddressParseError;

impl Error for NonFungibleIndexAddressParseError {}

impl Display for NonFungibleIndexAddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid non-fungible index address string")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_a_display_string() {
        let address = NonFungibleIndexAddress::new(
            ResourceAddress::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaA").unwrap(),
            123,
        );
        let display = address.to_string();
        let parsed = NonFungibleIndexAddress::from_str(&display).unwrap();
        assert_eq!(address, parsed);
    }
}
