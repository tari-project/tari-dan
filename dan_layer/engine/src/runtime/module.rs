//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::services::template_provider::TemplateProvider;

use crate::{packager::LoadedTemplate, runtime::StateTracker};

pub trait RuntimeModule<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>: Send + Sync {
    fn on_runtime_call(
        &self,
        _track: &StateTracker<TTemplateProvider>,
        _call: &'static str,
    ) -> Result<(), RuntimeModuleError> {
        Ok(())
    }
    // Add more runtime "hooks"
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum RuntimeModuleError {
    #[error("Todo")]
    Todo,
}
