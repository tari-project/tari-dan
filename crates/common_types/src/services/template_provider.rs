//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib_types::{Hash32, TemplateAddress, TemplateMetadataHash, crypto::RistrettoPublicKeyBytes};

use crate::Epoch;

pub trait TemplateProvider: Send + Sync + Clone + 'static {
    type Template;
    type Error: std::error::Error + Sync + Send + 'static;

    fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error>;
    fn has_template(&self, address: &TemplateAddress) -> Result<bool, Self::Error> {
        Ok(self.get_template(address)?.is_some())
    }
}

pub trait TemplateMetadataProvider: TemplateProvider {
    fn get_template_metadata(&self, id: &TemplateAddress) -> Result<Option<TemplateProviderMetadata>, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct TemplateProviderMetadata {
    pub author: RistrettoPublicKeyBytes,
    pub binary_hash: Hash32,
    pub epoch: Epoch,
    pub metadata_hash: Option<TemplateMetadataHash>,
}
