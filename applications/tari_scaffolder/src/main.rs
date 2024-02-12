//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

mod cli;
mod command;
mod generators;

use std::fs;

use tari_dan_engine::{
    template::TemplateModuleLoader,
    wasm::{compile::compile_template, WasmModule},
};

use crate::{
    cli::Cli,
    generators::{
        liquid::{LiquidGenerator, LiquidTemplate},
        CodeGenerator,
        GeneratorOpts,
        TemplateDefinition,
    },
};

fn main() -> anyhow::Result<()> {
    let cli = Cli::init();

    let mut opts = GeneratorOpts {
        output_path: "output/".into(),
        liquid: None,
    };

    if let Some(config_file) = &cli.generator_config_file {
        let config = fs::read_to_string(config_file)?;
        opts = serde_json::from_str(&config)?;
    }

    if let Some(output_path) = cli.output_path {
        opts.output_path = output_path;
    }

    if let Some(ref mut opts) = opts.liquid {
        for (k, v) in cli.data.unwrap_or_default() {
            opts.variables.insert(k, serde_json::Value::String(v));
        }
    }

    if cli.clean && fs::remove_dir_all(&opts.output_path).is_err() {
        println!("Failed to clean output directory");
    }

    match cli.command {
        command::Command::Scaffold(scaffold) => {
            println!("Scaffolding wasm at {:?}", scaffold.wasm_path);
            let wasm = if scaffold.wasm_path.extension() == Some("wasm".as_ref()) {
                WasmModule::from_code(fs::read(&scaffold.wasm_path).unwrap())
            } else {
                compile_template(&scaffold.wasm_path, &[]).unwrap()
            };

            let loaded_template = wasm.load_template().unwrap();
            let template: TemplateDefinition = loaded_template.into();
            match cli.generator {
                generators::GeneratorType::RustTemplateCli => {
                    LiquidGenerator::new(LiquidTemplate::RustCli, opts).generate(&template)?
                },
            }
        },
    }

    Ok(())
}
