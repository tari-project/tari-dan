//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_bor::{BorTag, Tagged};
use tari_template_abi::{EngineOp, call_engine};
use tari_template_lib_types::{BinaryTag, ComponentAddress, ResourceAddress};

use crate::args::{AddressAllocationInvokeArg, InvokeResult};

/// Represents an allocation of an address for a component or resource.
pub type AddressAllocationId = u32;

const COMPONENT_ALLOC_TAG: u64 = BinaryTag::AllocatedComponentAddress.as_u64();

/// Represents an allocation of an address for a component.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq, Hash)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct ComponentAddressAllocation(BorTag<AddressAllocationId, COMPONENT_ALLOC_TAG>);

impl Tagged for ComponentAddressAllocation {
    const TAG: u64 = COMPONENT_ALLOC_TAG;
}

impl ComponentAddressAllocation {
    /// Creates a new `ComponentAddressAllocation` with the given ID.
    /// For internal use.
    pub fn new(id: AddressAllocationId) -> Self {
        Self(BorTag::new(id))
    }

    /// Returns the ID of the address allocation.
    pub fn id(&self) -> AddressAllocationId {
        self.0.into_inner()
    }

    /// Retrieves the allocated address for the component.
    pub fn get_address(&self) -> ComponentAddress {
        let resp: InvokeResult = call_engine(
            EngineOp::AddressAllocationInvoke,
            &AddressAllocationInvokeArg::GetAddress(self.id()),
        );

        resp.decode()
            .expect("Failed to decode ComponentAddressAllocation response")
    }
}

const RESOURCE_TAG: u64 = BinaryTag::AllocatedResourceAddress.as_u64();

/// Represents an allocation of an address for a resource.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq, Hash)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct ResourceAddressAllocation(BorTag<AddressAllocationId, RESOURCE_TAG>);

impl ResourceAddressAllocation {
    /// Creates a new `ResourceAddressAllocation` with the given ID.
    /// For internal use.
    pub fn new(id: AddressAllocationId) -> Self {
        Self(BorTag::new(id))
    }

    /// Returns the ID of the address allocation.
    pub fn id(&self) -> AddressAllocationId {
        self.0.into_inner()
    }

    /// Retrieves the allocated address for the resource.
    pub fn get_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(
            EngineOp::AddressAllocationInvoke,
            &AddressAllocationInvokeArg::GetAddress(self.id()),
        );

        resp.decode()
            .expect("Failed to decode ResourceAddressAllocation response")
    }
}
