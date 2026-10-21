//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::published_template::{PublishedTemplate, PublishedTemplateAddress};
use tari_ootle_common_types::{
    SubstateAddress,
    SubstateVersion,
    optional::Optional,
    services::template_provider::TemplateProvider,
};
use tari_ootle_storage::StorageError;
use tari_template_lib_types::TemplateAddress;

use crate::{RocksDbStateStore, column_families::substate::SubstateCf};

impl<TAddr: Send + Sync + 'static> TemplateProvider for RocksDbStateStore<TAddr> {
    type Error = StorageError;
    type Template = PublishedTemplate;

    fn get_template(&self, id: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
        const OPERATION: &str = "RocksDbReadOnlyStateStore::get_template";
        let tx = self.read_view();
        let cf = tx.db().cf(SubstateCf)?;
        let address = template_address_to_substate_address(*id);
        let Some(substate) = cf.get(&address, OPERATION).optional()? else {
            return Ok(None);
        };
        let Some(value) = substate.into_substate_value() else {
            return Ok(None);
        };

        let template = value.into_template().ok_or_else(|| StorageError::DataInconsistency {
            details: format!("Template substate does not contain a published template: {}", address),
        })?;

        Ok(Some(template))
    }

    fn has_template(&self, id: &TemplateAddress) -> Result<bool, Self::Error> {
        const OPERATION: &str = "RocksDbReadOnlyStateStore::has_template";
        let tx = self.read_view();
        let cf = tx.db().cf(SubstateCf)?;
        let address = template_address_to_substate_address(*id);
        let exists = cf.exists(&address, OPERATION)?;
        Ok(exists)
    }
}

fn template_address_to_substate_address(address: TemplateAddress) -> SubstateAddress {
    let address = PublishedTemplateAddress::from_hash(address);
    SubstateAddress::from_object_key(address.as_object_key(), SubstateVersion::ZERO)
}
