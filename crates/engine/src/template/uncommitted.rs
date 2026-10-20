//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_template_lib::types::TemplateAddress;

use crate::template::LoadedTemplate;

/// The templates published in the uncommitted blocks of the chain one execution builds on.
pub trait UncommittedTemplates: Send + Sync {
    /// The template published at `address`, or the error loading it, if an uncommitted block of the chain publishes
    /// it.
    fn get_template(&self, address: &TemplateAddress) -> Option<Result<LoadedTemplate, String>>;
}

/// No uncommitted blocks, for an execution against committed state alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUncommittedTemplates;

impl UncommittedTemplates for NoUncommittedTemplates {
    fn get_template(&self, _address: &TemplateAddress) -> Option<Result<LoadedTemplate, String>> {
        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionTemplateProviderError<E> {
    #[error(transparent)]
    Committed(E),
    #[error("Failed to load uncommitted template {address}: {details}")]
    Uncommitted { address: TemplateAddress, details: String },
}

/// Resolves templates for one execution from the uncommitted blocks of its chain, then from committed state through
/// `inner`.
pub struct ExecutionTemplateProvider<TProvider> {
    inner: Arc<TProvider>,
    uncommitted: Arc<dyn UncommittedTemplates>,
}

impl<TProvider> ExecutionTemplateProvider<TProvider> {
    pub fn new(inner: Arc<TProvider>, uncommitted: Arc<dyn UncommittedTemplates>) -> Self {
        Self { inner, uncommitted }
    }
}

impl<TProvider> Clone for ExecutionTemplateProvider<TProvider> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            uncommitted: self.uncommitted.clone(),
        }
    }
}

impl<TProvider> TemplateProvider for ExecutionTemplateProvider<TProvider>
where TProvider: TemplateProvider<Template = LoadedTemplate>
{
    type Error = ExecutionTemplateProviderError<TProvider::Error>;
    type Template = LoadedTemplate;

    fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
        match self.uncommitted.get_template(address) {
            Some(Ok(template)) => Ok(Some(template)),
            Some(Err(details)) => Err(ExecutionTemplateProviderError::Uncommitted {
                address: *address,
                details,
            }),
            None => self
                .inner
                .get_template(address)
                .map_err(ExecutionTemplateProviderError::Committed),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, convert::Infallible};

    use tari_template_builtin::{ACCOUNT_TEMPLATE_ADDRESS, get_template_builtin};

    use super::*;
    use crate::wasm::WasmModule;

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

    struct Uncommitted(HashMap<TemplateAddress, Result<LoadedTemplate, String>>);

    impl UncommittedTemplates for Uncommitted {
        fn get_template(&self, address: &TemplateAddress) -> Option<Result<LoadedTemplate, String>> {
            self.0.get(address).cloned()
        }
    }

    fn provider() -> ExecutionTemplateProvider<Committed> {
        let template = WasmModule::load_template_from_code(get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS)).unwrap();
        let uncommitted = Uncommitted(HashMap::from([
            (UNCOMMITTED, Ok(template.clone())),
            (UNLOADABLE, Err("invalid module".to_string())),
        ]));
        ExecutionTemplateProvider::new(Arc::new(Committed(template)), Arc::new(uncommitted))
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
            Err(ExecutionTemplateProviderError::Uncommitted {
                address: UNLOADABLE,
                ..
            })
        ));
    }
}
