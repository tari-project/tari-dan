//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{
    resource::Resource,
    substate::{Substate, SubstateAddress},
};
use tari_template_lib::{constants::PUBLIC_IDENTITY_RESOURCE, prelude::ResourceType};

use crate::state_store::{StateStoreError, StateWriter};

pub fn bootstrap_state<T: StateWriter>(state_db: &mut T) -> Result<(), StateStoreError> {
    let address = SubstateAddress::Resource(PUBLIC_IDENTITY_RESOURCE);
    state_db.set_state(
        &address,
        Substate::new(0, Resource::new(ResourceType::NonFungible, Default::default())),
    )?;

    Ok(())
}
