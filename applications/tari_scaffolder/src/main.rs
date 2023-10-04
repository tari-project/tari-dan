mod cli;
mod command;

use std::{fs, path::Path};

use convert_case::{Case, Casing};
use liquid::model::Value;
use tari_dan_engine::{
    packager::{LoadedTemplate, TemplateModuleLoader},
    wasm::compile::compile_template,
};

use crate::{cli::Cli, LoadedTemplate::Wasm};

fn main() {
    let cli = Cli::init();

    if cli.clean {
        if fs::remove_dir_all(&cli.output_path).is_err() {
            println!("Failed to clean output directory");
        }
    }

    match cli.command {
        command::Command::Scaffold(scaffold) => {
            println!("Scaffolding wasm at {:?}", scaffold.wasm_path);

            let wasm = compile_template(scaffold.wasm_path, &[]).unwrap();

            let loaded_template = wasm.load_template().unwrap();
            dbg!(&loaded_template);
            generate(&loaded_template, cli.output_path.as_ref())
        },
    }
    // let config_path = cli.common.config_path();
    // let cfg = load_configuration(config_path, true, &cli)?;
    // let config = ApplicationConfig::load_from(&cfg)?;
    // println!("Starting validator node on network {}", config.network);

    println!("Hello, world!");
}

fn generate(template: &LoadedTemplate, output_path: &Path) {
    fs::create_dir_all(output_path.join("src")).unwrap();
    fs::write(
        output_path.join("Cargo.toml"),
        include_bytes!("./template/Cargo.toml.liquid"),
    )
    .unwrap();
    fs::write(
        output_path.join("src/main.rs"),
        replace_tokens(include_str!("./template/src/main.rs.liquid"), template),
    )
    .unwrap();
    fs::write(
        output_path.join("src/cli.rs"),
        replace_tokens(include_str!("./template/src/cli.rs.liquid"), template),
    )
    .unwrap();
    fs::write(
        output_path.join("src/daemon_client.rs"),
        replace_tokens(include_str!("./template/src/daemon_client.rs.liquid"), template),
    )
    .unwrap();
    // todo!()
}

fn replace_tokens(in_file: &str, loaded_template: &LoadedTemplate) -> String {
    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .unwrap()
        .parse(in_file)
        .unwrap();

    let mut globals = liquid::object!({
        "commands": [
        ]
    });

    match loaded_template {
        Wasm(loaded_wasm_template) => {
            for f in loaded_wasm_template.template_def().functions.iter() {
                let arr = globals.get_mut("commands").unwrap().as_array_mut().unwrap();
                let mut args = vec![];
                for a in &f.arguments {
                    args.push(liquid::object!({
                        "name": a.name
                    }));
                }
                arr.push(Value::Object(liquid::object!({
                    "name": f.name,
                    "title": f.name.to_case(Case::UpperCamel),
                    "args" : args
                })));
            }
        },
        _ => {},
    }
    dbg!(&globals);
    template.render(&mut globals).unwrap()
}
