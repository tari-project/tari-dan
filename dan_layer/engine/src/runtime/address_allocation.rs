//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{substate::SubstateId, TemplateAddress};
use tari_template_lib::models::ComponentAddress;

#[derive(Debug, Clone)]
pub struct AllocatedAddress {
    template_address: TemplateAddress,
    address: SubstateId,
}

impl AllocatedAddress {
    pub fn new(template_address: TemplateAddress, address: SubstateId) -> Self {
        Self {
            template_address,
            address,
        }
    }

    pub fn address(&self) -> &SubstateId {
        &self.address
    }

    pub fn template_address(&self) -> &TemplateAddress {
        &self.template_address
    }
}

impl TryFrom<AllocatedAddress> for ComponentAddress {
    type Error = SubstateId;

    fn try_from(value: AllocatedAddress) -> Result<Self, Self::Error> {
        value.address.try_into()
    }
}
