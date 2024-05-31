//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use serde::Deserialize;
use tari_dan_engine::{abi::TemplateDef, template::LoadedTemplate};

pub mod liquid;

pub struct TemplateDefinition {
    pub name: String,
    pub template: TemplateDef,
}

impl From<LoadedTemplate> for TemplateDefinition {
    fn from(loaded_template: LoadedTemplate) -> Self {
        Self {
            name: loaded_template.template_name().to_string(),
            template: loaded_template.template_def().clone(),
        }
    }
}

pub trait CodeGenerator {
    fn generate(&self, template: &TemplateDefinition) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneratorOpts {
    pub output_path: PathBuf,
    pub liquid: Option<LiquidGeneratorOpts>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiquidGeneratorOpts {
    #[serde(default)]
    pub skip_format: bool,
    pub variables: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
pub enum GeneratorType {
    RustTemplateCli,
}

impl FromStr for GeneratorType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rust" => Ok(GeneratorType::RustTemplateCli),
            "rust-template-cli" => Ok(GeneratorType::RustTemplateCli),
            _ => Err(anyhow::anyhow!("Invalid generator type")),
        }
    }
}
