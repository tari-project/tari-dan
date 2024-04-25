//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod generators;

use std::{
    collections::HashMap,
    fs::{self, create_dir_all, File},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::anyhow;
use clap::Parser;
use generators::GeneratorType;
use serde_json::{json, Value};
use tari_dan_engine::{template::TemplateModuleLoader, wasm::WasmModule};

use crate::generators::{
    liquid::{LiquidGenerator, LiquidTemplate},
    CodeGenerator,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
enum Cli {
    Generate(GenerateArgs),
    Build(BuildArgs),
    Publish(PublishArgs),
    ListRegisteredTemplates(ListRegisteredTemplatesArgs),
    Scaffold(ScaffoldArgs),
}

#[derive(clap::Args, Debug)]
struct GenerateArgs {
    #[clap(long, short = 'o')]
    pub output_path: Option<PathBuf>,
    #[clap(long, short = 't')]
    template: String,
    #[clap(long, short = 'n')]
    name: Option<String>,
}

#[derive(clap::Args, Debug)]
struct BuildArgs {
    #[clap(long, short = 'p')]
    path: Option<PathBuf>,
    #[clap(long)]
    profile: Option<String>,
}

#[derive(clap::Args, Debug, Clone)]
struct PublishArgs {
    #[clap(long, short = 'p')]
    path: Option<PathBuf>,
    #[clap(long)]
    profile: Option<String>,
    #[clap(long, short = 'j')]
    dan_testing_jrpc_url: String,
}

impl From<PublishArgs> for BuildArgs {
    fn from(val: PublishArgs) -> BuildArgs {
        BuildArgs {
            path: val.path,
            profile: val.profile,
        }
    }
}

#[derive(clap::Args, Debug)]
struct ListRegisteredTemplatesArgs {
    #[clap(long, short = 'j')]
    dan_testing_jrpc_url: String,
}

#[derive(clap::Args, Debug)]
struct ScaffoldArgs {
    #[clap(long, short = 's')]
    pub clean: bool,
    #[clap(long, short = 'o')]
    pub output_path: Option<PathBuf>,
    #[clap(long, short = 'g')]
    pub generator: GeneratorType,
    #[clap(long, short = 'd', alias = "data", value_parser = parse_hashmap)]
    pub data: Option<HashMap<String, String>>,
    #[clap(long, short = 'c', alias = "config")]
    pub generator_config_file: Option<PathBuf>,
    #[clap(long, short = 'a')]
    template_address: String,
    #[clap(long, short = 'j')]
    dan_testing_jrpc_url: String,
}

fn parse_hashmap(input: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in input.split(',') {
        let mut parts = pair.splitn(2, ':');
        let key = parts
            .next()
            .ok_or(anyhow!(
                "The `data` input is wrong. Values should be comma separated and each value has to be in format \
                 `string:string`."
            ))?
            .to_string();
        let value = parts
            .next()
            .ok_or(anyhow!(
                "The `data` input is wrong. Values should be comma separated and each value has to be in format \
                 `string:string`."
            ))?
            .to_string();
        map.insert(key, value);
    }
    Ok(map)
}

fn generate(args: GenerateArgs) -> anyhow::Result<()> {
    let output = Command::new("cargo")
        .arg("generate")
        .arg("-n")
        .arg(args.name.unwrap_or(args.template.clone()))
        .arg("https://github.com/tari-project/wasm-template.git")
        .arg(args.template)
        .current_dir(args.output_path.unwrap_or(".".into()))
        .output()
        .expect("failed to execute process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{}", stderr);
    }
    Ok(())
}

fn build(args: BuildArgs) -> anyhow::Result<()> {
    let path = args.path.unwrap_or(".".into());
    let path = Path::new(&path).join("package");
    let profile = match args.profile {
        Some(debug) if debug == "debug" => "dev".to_string(),
        None => "release".to_string(),
        Some(profile) => profile,
    };
    let output = Command::new("cargo")
        .arg("build")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--profile")
        .arg(profile)
        .current_dir(path)
        .output()
        .expect("failed to execute process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{}", stderr);
    }
    Ok(())
}

fn ensure_prefix(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("http://{}", url)
    }
}

fn search_wasm_files(directory: &PathBuf) -> Option<String> {
    let path = Path::new(directory);
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() {
                    if let Some(extension) = entry.path().extension() {
                        if extension == "wasm" {
                            if let Some(file_name) = entry.file_name().to_str() {
                                return Some(file_name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

async fn publish(args: PublishArgs) -> anyhow::Result<()> {
    build(args.clone().into())?;

    let directory = Path::new(&args.path.unwrap_or(".".into()))
        .join("package")
        .join("target")
        .join("wasm32-unknown-unknown")
        .join(args.profile.unwrap_or("release".to_string()));
    let wasm_name = search_wasm_files(&directory).ok_or(anyhow!("No wasm file found in the directory"))?;

    let file_path = directory.join(wasm_name.clone());
    let jrpc_url = ensure_prefix(args.dan_testing_jrpc_url.as_str());
    let url = format!("{}/upload_template", jrpc_url);

    let file_fs = fs::read(file_path).expect("failed to read file");
    let file = reqwest::multipart::Part::bytes(file_fs.clone()).file_name(wasm_name);
    let form = reqwest::multipart::Form::new().part("file", file);

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .multipart(form)
        .send()
        .await
        .expect("failed to send request");

    if !response.status().is_success() {
        return Err(anyhow!("Failed to upload template.\n{:?}", response));
    }

    let request = json!({
        "jsonrpc": "2.0",
        "method": "mine",
        "params": [4],
        "id": 1
    });

    let response = reqwest::Client::new()
        .post(jrpc_url)
        .json(&request)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        println!("Failed to mine");
        println!("{:?}", response);
    }
    Ok(())
}

async fn list_registered_templates(args: ListRegisteredTemplatesArgs) -> anyhow::Result<()> {
    let jrpc_url = ensure_prefix(args.dan_testing_jrpc_url.as_str());

    let request = json!({
        "jsonrpc": "2.0",
        "method": "get_templates",
        "params": [0],
        "id": 1
    });

    let response = reqwest::Client::new()
        .post(jrpc_url.clone())
        .json(&request)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await?;
    let result = response.json::<Value>().await?;
    println!("Templates:");
    for templates in result["result"]["templates"]
        .as_array()
        .ok_or(anyhow!("Template address is not an array"))?
    {
        let name = templates["name"].as_str().unwrap_or("Name was not returned");
        let address_bytes = templates["address"]
            .as_array()
            .ok_or(anyhow!("Address is not an array"))?
            .iter()
            .map(|v| {
                v.as_u64()
                    .ok_or(anyhow!("Address is not a u64"))
                    .and_then(|v| u8::try_from(v).map_err(|_| anyhow!("Address is not a u8")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let address = hex::encode(address_bytes.clone());
        println!("Template: {} Address: {}", name, address);
    }
    Ok(())
}

async fn scaffold(args: ScaffoldArgs) -> anyhow::Result<()> {
    let mut opts = generators::GeneratorOpts {
        output_path: "output/".into(),
        liquid: Some(generators::LiquidGeneratorOpts {
            skip_format: false,
            variables: vec![("template_address".to_string(), Value::String(args.template_address.clone()))]
                .into_iter()
                .collect(),
        }),
    };

    if let Some(config_file) = &args.generator_config_file {
        let config = fs::read_to_string(config_file)?;
        opts = serde_json::from_str(&config)?;
    }

    if let Some(output_path) = args.output_path {
        opts.output_path = output_path;
    }

    if args.clean && fs::remove_dir_all(&opts.output_path).is_err() {
        println!("Failed to clean output directory");
    }

    let jrpc_url = ensure_prefix(args.dan_testing_jrpc_url.as_str());
    let request = json!({
        "jsonrpc" : "2.0",
        "method": "get_template",
        "params": [hex::decode(args.template_address)?],
        "id": 1
    });
    let response = reqwest::Client::new()
        .post(jrpc_url)
        .json(&request)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await?;
    let response = response.json::<Value>().await?;
    let name: &str = response["result"]["registration_metadata"]["name"]
        .as_str()
        .unwrap_or("unnamed.wasm");
    let url = response["result"]["registration_metadata"]["url"]
        .as_str()
        .ok_or(anyhow!("No url found"))?;
    let file_response = reqwest::get(url).await?;
    let wasm_file_path = opts.output_path.join(name);
    create_dir_all(wasm_file_path.parent().ok_or(anyhow!("Wasm file path is not valid."))?)?;
    let mut dest = File::create(wasm_file_path.clone())?;
    let content = file_response.bytes().await?;
    std::io::copy(&mut content.as_ref(), &mut dest)?;
    drop(dest);

    if let Some(ref mut opts) = opts.liquid {
        for (k, v) in args.data.unwrap_or_default() {
            opts.variables.insert(k, serde_json::Value::String(v));
        }
    }

    println!("Scaffolding wasm at {:?}", wasm_file_path);
    let f = fs::read(&wasm_file_path)?;
    let wasm = WasmModule::from_code(f);
    let loaded_template = wasm.load_template()?;
    let template = loaded_template.into();
    match args.generator {
        GeneratorType::RustTemplateCli => LiquidGenerator::new(LiquidTemplate::RustCli, opts).generate(&template)?,
    };
    Ok(())
}

#[tokio::main]
async fn main() {
    let result = match Cli::parse() {
        Cli::Generate(args) => generate(args),
        Cli::Build(args) => build(args),
        Cli::Publish(args) => publish(args).await,
        Cli::ListRegisteredTemplates(args) => list_registered_templates(args).await,
        Cli::Scaffold(args) => scaffold(args).await,
    };
    if result.is_err() {
        println!("Error: {}", result.err().unwrap());
    }
}
