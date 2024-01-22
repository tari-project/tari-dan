//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    pub struct AddressAllocationTest {}

    impl AddressAllocationTest {
        pub fn create() -> (Component<Self>, ComponentAddress) {
            let allocation = CallerContext::allocate_component_address();
            let address = allocation.address().clone();
            (
                Component::new(Self {}).with_address_allocation(allocation).create(),
                address,
            )
        }

        pub fn drop_allocation() {
            let _allocation = CallerContext::allocate_component_address();
        }
    }
}
