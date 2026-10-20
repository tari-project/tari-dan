//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use tari_engine_types::published_template::PublishedTemplate;
use tari_ootle_common_types::services::template_provider::TemplateProvider;
use tari_template_lib::types::TemplateAddress;

use crate::{template::LoadedTemplate, wasm::WasmModule};

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
    /// Returns a copy holding `published` at `address` as well, with no misses recorded.
    pub fn with_published(&self, address: TemplateAddress, published: &PublishedTemplate) -> Self {
        let mut templates = (*self.templates).clone();
        templates.insert(
            address,
            WasmModule::load_template_from_code(published.binary.as_slice()).map_err(|e| e.to_string()),
        );
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
