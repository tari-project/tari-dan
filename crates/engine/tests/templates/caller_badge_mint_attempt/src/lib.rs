//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

/// Attempts to obtain a real token of a caller-badge resource. Nothing exists at either badge resource
/// address, so there is no resource to mint from and therefore no path from a caller badge to a `Proof`.
#[template]
mod mint_attempt {
    use super::*;

    pub struct BadgeMintAttempt;

    impl BadgeMintAttempt {
        pub fn mint_caller_component_badge() -> Bucket {
            ResourceManager::get(CALLER_COMPONENT_RESOURCE_ADDRESS).mint_non_fungible(
                NonFungibleId::from_u256([0u8; 32]),
                &(),
                &(),
            )
        }

        pub fn mint_direct_caller_template_badge() -> Bucket {
            ResourceManager::get(DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS).mint_non_fungible(
                NonFungibleId::from_u256([0u8; 32]),
                &(),
                &(),
            )
        }
    }
}
