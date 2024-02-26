//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod snake_case;

use std::fs;

use convert_case::{Case, Casing};
use tari_dan_engine::abi;

use crate::generators::{CodeGenerator, GeneratorOpts, TemplateDefinition};

pub enum LiquidTemplate {
    RustCli,
}

impl LiquidTemplate {
    const fn get_template(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            LiquidTemplate::RustCli => &[
                (
                    "Cargo.toml",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/liquid_templates/rust_cli/Cargo.toml.liquid"
                    )),
                ),
                (
                    ".gitignore",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/liquid_templates/rust_cli/.gitignore.liquid"
                    )),
                ),
                (
                    "src/main.rs",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/liquid_templates/rust_cli/src/main.rs.liquid"
                    )),
                ),
                (
                    "src/cli.rs",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/liquid_templates/rust_cli/src/cli.rs.liquid"
                    )),
                ),
                (
                    "src/daemon_client.rs",
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/liquid_templates/rust_cli/src/daemon_client.rs.liquid"
                    )),
                ),
            ],
        }
    }
}

pub struct LiquidGenerator {
    template: LiquidTemplate,
    opts: GeneratorOpts,
}

impl LiquidGenerator {
    pub const fn new(template: LiquidTemplate, opts: GeneratorOpts) -> Self {
        Self { template, opts }
    }

    fn build_vars(&self, template: &TemplateDefinition) -> liquid_core::Object {
        let opts = self.opts.liquid.as_ref().unwrap();

        let mut globals = liquid::object!({
            "template_name": &template.name,
            "commands": []
        });
        globals.extend(
            opts.variables
                .iter()
                .map(|(k, v)| (k.clone().into(), json_value_to_liquid_value(v.clone()))),
        );

        for f in template.template.functions() {
            let mut args = vec![];
            let mut is_method = false;
            let mut requires_buckets = false;
            let mut bucket_output = false;
            for a in &f.arguments {
                args.push(liquid::object!({
                    "name": a.name,
                    "arg_type": a.arg_type.to_string(),
                }));
                if a.arg_type.to_string() == "Bucket" {
                    requires_buckets = true;
                }
                if a.name == "self" {
                    is_method = true;
                }
            }

            if let abi::Type::Other { name } = &f.output {
                if name == "Bucket" {
                    bucket_output = true;
                }
            }

            let arr = globals.get_mut("commands").unwrap().as_array_mut().unwrap();
            arr.push(liquid_core::Value::Object(liquid::object!({
                "name": f.name,
                "title": f.name.to_case(Case::UpperCamel),
                "args" : args,
                "is_method": is_method,
                "is_mut": f.is_mut,
                "output": f.output.to_string(),
                "requires_buckets": requires_buckets,
                "bucket_output": bucket_output,
            })));
        }
        globals
    }
}

impl CodeGenerator for LiquidGenerator {
    fn generate(&self, template: &TemplateDefinition) -> anyhow::Result<()> {
        let opts = &self.opts;
        fs::create_dir_all(opts.output_path.join("src"))?;

        let templates = self.template.get_template();

        let vars = self.build_vars(template);

        for (out_file, content) in templates {
            fs::write(opts.output_path.join(out_file), replace_tokens(content, &vars)?)?;
        }

        if !self.opts.liquid.as_ref().unwrap().skip_format {
            std::process::Command::new("cargo")
                .args(["fmt"])
                .current_dir(&opts.output_path)
                .status()?;
        }

        Ok(())
    }
}

fn replace_tokens(in_file: &str, globals: &liquid_core::Object) -> anyhow::Result<String> {
    let template = liquid::ParserBuilder::with_stdlib()
        .filter(snake_case::SnakeCase)
        .build()?
        .parse(in_file)?;

    let built_template = template.render(globals)?;
    Ok(built_template)
}

fn json_value_to_liquid_value(value: serde_json::Value) -> liquid_core::Value {
    match value {
        serde_json::Value::Null => liquid_core::Value::Nil,
        serde_json::Value::Bool(b) => liquid_core::Value::scalar(b),
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                liquid_core::Value::scalar(n.as_i64().unwrap())
            } else {
                liquid_core::Value::scalar(n.as_f64().unwrap())
            }
        },
        serde_json::Value::String(s) => liquid_core::Value::scalar(s),
        serde_json::Value::Array(a) => liquid_core::Value::Array(
            a.into_iter()
                .map(json_value_to_liquid_value)
                .collect::<Vec<liquid_core::Value>>(),
        ),
        serde_json::Value::Object(o) => liquid_core::Value::Object(
            o.into_iter()
                .map(|(k, v)| (k.into(), json_value_to_liquid_value(v)))
                .collect::<liquid_core::Object>(),
        ),
    }
}
