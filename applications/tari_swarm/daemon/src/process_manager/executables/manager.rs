//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use futures::{stream::FuturesUnordered, StreamExt};
use tokio::process::{Child, Command};

use super::executable::Executable;
use crate::config::{ExecutableConfig, InstanceType};

pub struct ExecutableManager {
    config: Vec<ExecutableConfig>,
    always_compile: bool,
    prepared: Vec<Executable>,
}

impl ExecutableManager {
    pub fn new(config: Vec<ExecutableConfig>, always_compile: bool) -> Self {
        Self {
            config,
            always_compile,
            prepared: Vec::new(),
        }
    }

    pub async fn prepare(&mut self) -> anyhow::Result<Executables<'_>> {
        log::info!("Preparing {} executables", self.config.len());
        self.prepared.clear();

        let mut tasks = FuturesUnordered::new();

        for exec in &self.config {
            if let Some(exec_path) = exec.get_executable_path() {
                if !self.always_compile && exec_path.exists() {
                    self.prepared.push(Executable {
                        instance_type: exec.instance_type,
                        path: exec_path,
                        env: exec.env.clone(),
                    });
                    continue;
                }

                let Some(ref compile) = exec.compile else {
                    return Err(anyhow!(
                        "Attempted to compile {} however no compile config was provided",
                        exec.instance_type
                    ));
                };

                log::info!(
                    "Compiling {} in working dir {}",
                    exec.instance_type,
                    compile.working_dir().display()
                );
                let mut child = cargo_build(
                    compile
                        .working_dir()
                        .canonicalize()
                        .context("working_dir does not exist")?,
                    &compile.package_name,
                )?;
                tasks.push(async move {
                    let status = child.wait().await?;
                    Ok::<_, anyhow::Error>((status, exec))
                });
            }
        }

        while let Some(output) = tasks.next().await {
            let (status, exec) = output?;
            if !status.success() {
                return Err(anyhow!("Failed to compile {:?}", exec.instance_type));
            }

            log::info!("🟢 Successfully compiled {}", exec.instance_type);

            let compile = exec
                .compile
                .as_ref()
                .expect("BUG: Compiled but compile config was None");

            let bin_path = compile
                .working_dir()
                .join(compile.target_dir())
                .join(&compile.package_name);

            self.prepared.push(Executable {
                instance_type: exec.instance_type,
                path: add_ext(&bin_path)
                    .canonicalize()
                    .with_context(|| anyhow!("The compiled binary at path '{}' does not exist.", bin_path.display()))?,
                env: exec.env.clone(),
            })
        }

        Ok(Executables {
            executables: &self.prepared,
        })
    }

    pub fn get_executable(&self, instance_type: InstanceType) -> Option<&Executable> {
        self.prepared.iter().find(|e| e.instance_type == instance_type)
    }
}

fn cargo_build<P: AsRef<Path>>(working_dir: P, package: &str) -> io::Result<Child> {
    Command::new("cargo")
        .args(["build", "--release", "--bin", package])
        .current_dir(working_dir)
        .spawn()
}

pub struct Executables<'a> {
    executables: &'a [Executable],
}

impl<'a> Executables<'a> {
    pub fn get(&self, instance_type: InstanceType) -> Option<&Executable> {
        self.executables.iter().find(|e| e.instance_type == instance_type)
    }
}

fn add_ext<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref().to_path_buf();

    if cfg!(windows) {
        path.with_extension(".exe")
    } else {
        path
    }
}
