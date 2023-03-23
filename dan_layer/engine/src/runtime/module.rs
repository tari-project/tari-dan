//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_engine_types::substate::{SubstateAddress, SubstateValue};

use crate::{packager::LoadedTemplate, runtime::StateTracker};

pub trait RuntimeModule<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>: Send + Sync {
    fn on_initialize(&self, _track: &StateTracker) -> Result<(), RuntimeModuleError> {
        Ok(())
    }

    fn on_runtime_call(
        &self,
        _track: &StateTracker<TTemplateProvider>,
        _call: &'static str,
    ) -> Result<(), RuntimeModuleError> {
        Ok(())
    }

    fn on_before_finalize(
        &self,
        _track: &StateTracker,
        _changes: &HashMap<SubstateAddress, SubstateValue>,
    ) -> Result<(), RuntimeModuleError> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeModuleError {
    #[error("BOR error: {0}")]
    Bor(#[from] tari_bor::BorError),
}
