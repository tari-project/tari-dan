use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use tari_engine_types::TemplateAddress;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct TemplateConfig {
    debug_replacements: Vec<String>,
}

impl TemplateConfig {
    pub fn debug_replacements(&self) -> HashMap<TemplateAddress, PathBuf> {
        let mut result = HashMap::new();
        for row in &self.debug_replacements {
            let parts: Vec<&str> = row.split('=').collect();
            let template_address = TemplateAddress::from_hex(parts[0]).expect("Not a valid template address");
            let path = PathBuf::from(parts[1]);
            result.insert(template_address, path);
        }
        result
    }
}
