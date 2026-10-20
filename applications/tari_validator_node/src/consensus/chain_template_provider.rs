//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::UncommittedChainTemplates;
use tari_engine::template::{LoadedTemplate, TemplateLoaderError};
use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_ootle_template_provider::UncommittedTemplateCache;
use tari_template_lib::types::TemplateAddress;

/// Resolves the templates of the chain one block builds on: those its uncommitted ancestors publish, compiled through
/// `uncommitted_cache`, then those in durably committed state through `committed`.
#[derive(Clone)]
pub struct ChainTemplateProvider<TCommitted> {
    committed: TCommitted,
    uncommitted_cache: UncommittedTemplateCache,
    chain: UncommittedChainTemplates,
}

impl<TCommitted> ChainTemplateProvider<TCommitted> {
    pub fn new(
        committed: TCommitted,
        uncommitted_cache: UncommittedTemplateCache,
        chain: UncommittedChainTemplates,
    ) -> Self {
        Self {
            committed,
            uncommitted_cache,
            chain,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainTemplateProviderError<E> {
    #[error(transparent)]
    Committed(E),
    #[error("Failed to load uncommitted template {address}: {source}")]
    Uncommitted {
        address: TemplateAddress,
        source: TemplateLoaderError,
    },
}

impl<TCommitted> TemplateProvider for ChainTemplateProvider<TCommitted>
where TCommitted: TemplateProvider<Template = LoadedTemplate>
{
    type Error = ChainTemplateProviderError<TCommitted::Error>;
    type Template = LoadedTemplate;

    fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
        let Some(published) = self.chain.get(address) else {
            return self
                .committed
                .get_template(address)
                .map_err(ChainTemplateProviderError::Committed);
        };
        self.uncommitted_cache
            .load(*address, published.binary.as_slice())
            .map(Some)
            .map_err(|source| ChainTemplateProviderError::Uncommitted {
                address: *address,
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, convert::Infallible, sync::Arc};

    use tari_engine::wasm::WasmModule;
    use tari_engine_types::published_template::PublishedTemplate;
    use tari_template_builtin::{ACCOUNT_TEMPLATE_ADDRESS, get_template_builtin};

    use super::*;

    const COMMITTED: TemplateAddress = TemplateAddress::from_array([1; 32]);
    const UNCOMMITTED: TemplateAddress = TemplateAddress::from_array([2; 32]);
    const UNLOADABLE: TemplateAddress = TemplateAddress::from_array([3; 32]);
    const ABSENT: TemplateAddress = TemplateAddress::from_array([4; 32]);

    #[derive(Clone)]
    struct Committed(LoadedTemplate);

    impl TemplateProvider for Committed {
        type Error = Infallible;
        type Template = LoadedTemplate;

        fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
            Ok((*address == COMMITTED).then(|| self.0.clone()))
        }
    }

    fn published(binary: &[u8]) -> Arc<PublishedTemplate> {
        Arc::new(PublishedTemplate {
            template_name: "test".try_into().unwrap(),
            author: Default::default(),
            binary: binary.to_vec().try_into().unwrap(),
            at_epoch: 0,
            metadata_hash: None,
        })
    }

    fn provider() -> ChainTemplateProvider<Committed> {
        let code = get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS);
        let chain = UncommittedChainTemplates::new(HashMap::from([
            (UNCOMMITTED, published(code)),
            (UNLOADABLE, published(b"not wasm")),
        ]));
        ChainTemplateProvider::new(
            Committed(WasmModule::load_template_from_code(code).unwrap()),
            UncommittedTemplateCache::new(u64::MAX),
            chain,
        )
    }

    #[test]
    fn a_committed_or_uncommitted_template_resolves() {
        let provider = provider();
        assert!(provider.get_template(&COMMITTED).unwrap().is_some());
        assert!(provider.get_template(&UNCOMMITTED).unwrap().is_some());
    }

    #[test]
    fn a_template_in_neither_is_not_found() {
        assert!(provider().get_template(&ABSENT).unwrap().is_none());
    }

    #[test]
    fn an_uncommitted_template_that_fails_to_load_fails_the_lookup() {
        assert!(matches!(
            provider().get_template(&UNLOADABLE),
            Err(ChainTemplateProviderError::Uncommitted {
                address: UNLOADABLE,
                ..
            })
        ));
    }
}
