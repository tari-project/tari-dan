//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{
    resource::Resource,
    substate::{Substate, SubstateAddress},
};
use tari_template_lib::{
    auth::{AccessRule, ResourceAccessRules},
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    crypto::RistrettoPublicKeyBytes,
    models::Metadata,
    prelude::{OwnerRule, ResourceType},
    resource::TOKEN_SYMBOL,
};

use crate::state_store::{StateStoreError, StateWriter};

pub fn bootstrap_state<T: StateWriter>(state_db: &mut T) -> Result<(), StateStoreError> {
    let address = SubstateAddress::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let mut metadata = Metadata::new();
    metadata.insert(TOKEN_SYMBOL, "ID".to_string());
    // Create the resource for badges
    state_db.set_state(
        &address,
        Substate::new(
            0,
            Resource::new(
                ResourceType::NonFungible,
                RistrettoPublicKeyBytes::default(),
                OwnerRule::None,
                ResourceAccessRules::deny_all(),
                metadata,
            ),
        ),
    )?;

    // Create the second layer tari resource
    let address = SubstateAddress::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let mut metadata = Metadata::new();
    // TODO: decide on symbol for L2 tari
    metadata.insert(TOKEN_SYMBOL, "tXTR2".to_string());
    state_db.set_state(
        &address,
        Substate::new(
            0,
            Resource::new(
                ResourceType::Confidential,
                RistrettoPublicKeyBytes::default(),
                OwnerRule::None,
                ResourceAccessRules::new()
                    .withdrawable(AccessRule::AllowAll)
                    .depositable(AccessRule::AllowAll),
                metadata,
            ),
        ),
    )?;

    Ok(())
}
