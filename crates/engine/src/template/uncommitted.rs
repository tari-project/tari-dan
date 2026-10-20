//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_template_lib::types::TemplateAddress;

use crate::template::LoadedTemplate;

/// Templates published in the uncommitted part of the chain a transaction executes on, which the executing validator
/// resolved before the execution, and the addresses the execution could resolve from neither those nor committed
/// state.
#[derive(Debug, Clone, Default)]
pub struct UncommittedTemplates {
    /// A template that fails to load keeps its error, so the execution fails to load it as it would from committed
    /// state.
    templates: Arc<HashMap<TemplateAddress, Result<LoadedTemplate, String>>>,
    misses: Arc<Mutex<HashSet<TemplateAddress>>>,
}

impl UncommittedTemplates {
    /// Returns a copy holding `template`, or the error loading it, at `address` as well, with no misses recorded.
    pub fn with_template(&self, address: TemplateAddress, template: Result<LoadedTemplate, String>) -> Self {
        let mut templates = (*self.templates).clone();
        templates.insert(address, template);
        Self {
            templates: Arc::new(templates),
            misses: Arc::default(),
        }
    }

    /// The addresses an execution using these templates could not resolve.
    pub fn take_misses(&self) -> HashSet<TemplateAddress> {
        std::mem::take(&mut *self.misses.lock().unwrap())
    }

    fn get(&self, address: &TemplateAddress) -> Option<&Result<LoadedTemplate, String>> {
        self.templates.get(address)
    }

    fn record_miss(&self, address: TemplateAddress) {
        self.misses.lock().unwrap().insert(address);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionTemplateProviderError<E> {
    #[error(transparent)]
    Committed(E),
    #[error("Failed to load uncommitted template {address}: {details}")]
    Uncommitted { address: TemplateAddress, details: String },
}

/// Resolves templates for one execution from committed state, through `inner`, and then from the uncommitted
/// templates.
///
/// Committed state is a prefix of the chain any block a validator executes builds on, and templates are immutable and
/// never destroyed, so a template either source holds is one the chain holds.
#[derive(Debug)]
pub struct ExecutionTemplateProvider<TProvider> {
    inner: Arc<TProvider>,
    uncommitted: UncommittedTemplates,
}

impl<TProvider> ExecutionTemplateProvider<TProvider> {
    pub fn new(inner: Arc<TProvider>, uncommitted: UncommittedTemplates) -> Self {
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
        if let Some(template) = self
            .inner
            .get_template(address)
            .map_err(ExecutionTemplateProviderError::Committed)?
        {
            return Ok(Some(template));
        }

        match self.uncommitted.get(address) {
            Some(Ok(template)) => Ok(Some(template.clone())),
            Some(Err(details)) => Err(ExecutionTemplateProviderError::Uncommitted {
                address: *address,
                details: details.clone(),
            }),
            None => {
                self.uncommitted.record_miss(*address);
                Ok(None)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

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

    fn provider() -> (ExecutionTemplateProvider<Committed>, UncommittedTemplates) {
        let template = WasmModule::load_template_from_code(get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS)).unwrap();
        let uncommitted = UncommittedTemplates::default()
            .with_template(UNCOMMITTED, Ok(template.clone()))
            .with_template(UNLOADABLE, Err("invalid module".to_string()));
        let provider = ExecutionTemplateProvider::new(Arc::new(Committed(template)), uncommitted.clone());
        (provider, uncommitted)
    }

    #[test]
    fn a_committed_or_uncommitted_template_resolves_without_a_miss() {
        let (provider, uncommitted) = provider();
        assert!(provider.get_template(&COMMITTED).unwrap().is_some());
        assert!(provider.get_template(&UNCOMMITTED).unwrap().is_some());
        assert!(uncommitted.take_misses().is_empty());
    }

    #[test]
    fn a_template_in_neither_is_not_found_and_recorded_as_a_miss() {
        let (provider, uncommitted) = provider();
        assert!(provider.get_template(&ABSENT).unwrap().is_none());
        assert_eq!(uncommitted.take_misses(), HashSet::from([ABSENT]));
        assert!(uncommitted.take_misses().is_empty());
    }

    #[test]
    fn an_uncommitted_template_that_fails_to_load_fails_the_lookup() {
        let (provider, uncommitted) = provider();
        assert!(matches!(
            provider.get_template(&UNLOADABLE),
            Err(ExecutionTemplateProviderError::Uncommitted {
                address: UNLOADABLE,
                ..
            })
        ));
        assert!(uncommitted.take_misses().is_empty());
    }
}
