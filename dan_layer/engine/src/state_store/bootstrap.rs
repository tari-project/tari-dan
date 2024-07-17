//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{
    resource::Resource,
    substate::{Substate, SubstateId},
};
use tari_template_lib::{
    auth::ResourceAccessRules,
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS},
    models::Metadata,
    prelude::{OwnerRule, ResourceType},
    resource::TOKEN_SYMBOL,
};

use crate::state_store::{memory::MemoryStateStore, AtomicDb, StateStoreError, StateWriter};

pub fn new_memory_store() -> MemoryStateStore {
    let state_db = MemoryStateStore::new();
    // unwrap: Memory state store is infallible
    let mut tx = state_db.write_access().unwrap();
    // Add shared global resources
    add_global_resources(&mut tx).unwrap();
    tx.commit().unwrap();
    state_db
}

/// These are implicitly included in every transaction. These are immutable and pledging them is not required.
fn add_global_resources<T: StateWriter>(state_db: &mut T) -> Result<(), StateStoreError> {
    let address = SubstateId::Resource(PUBLIC_IDENTITY_RESOURCE_ADDRESS);
    let mut metadata = Metadata::new();
    metadata.insert(TOKEN_SYMBOL, "ID".to_string());
    // Create the resource for badges
    state_db.set_state(
        &address,
        Substate::new(
            0,
            Resource::new(
                ResourceType::NonFungible,
                None,
                OwnerRule::None,
                ResourceAccessRules::deny_all(),
                metadata,
                None,
                None,
            ),
        ),
    )?;

    // Create the second layer tari resource
    let address = SubstateId::Resource(CONFIDENTIAL_TARI_RESOURCE_ADDRESS);
    let mut metadata = Metadata::new();
    metadata.insert(TOKEN_SYMBOL, "XTR".to_string());
    state_db.set_state(
        &address,
        Substate::new(
            0,
            Resource::new(
                ResourceType::Confidential,
                None,
                OwnerRule::None,
                ResourceAccessRules::new(),
                metadata,
                None,
                None,
            ),
        ),
    )?;

    Ok(())
}
