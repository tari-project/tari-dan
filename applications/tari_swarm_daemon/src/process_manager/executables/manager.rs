//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, env, io, path::Path};

use anyhow::{Context, anyhow};
use futures::{StreamExt, stream::FuturesUnordered};
use tokio::process::{Child, Command};

use super::executable::Executable;
use crate::config::{ExecutableConfig, InstanceType};

pub struct ExecutableManager {
    config: Vec<ExecutableConfig>,
    always_compile: bool,
    prepared: Vec<Executable>,
    /// Instance types whose executable config changed since they were prepared, mapped to whether their compile
    /// settings changed. These are prepared again the next time an instance of that type starts.
    stale: HashMap<InstanceType, bool>,
}

impl ExecutableManager {
    pub fn new(config: Vec<ExecutableConfig>, always_compile: bool) -> Self {
        Self {
            config,
            always_compile,
            prepared: Vec::new(),
            stale: HashMap::new(),
        }
    }

    pub async fn prepare_all(&mut self) -> anyhow::Result<Executables<'_>> {
        log::info!("Compiling {} executables", self.config.len());
        self.prepared.clear();
        self.stale.clear();

        let mut tasks = self
            .config
            .iter()
            .filter(|exec| exec.get_executable_path().is_some())
            .map(|exec| prepare(exec, self.always_compile))
            .collect::<FuturesUnordered<_>>();

        while let Some(executable) = tasks.next().await {
            self.prepared.push(executable?);
        }

        Ok(Executables {
            executables: &self.prepared,
        })
    }

    /// Replaces the executable config with one reloaded from the config file. Executables whose config changed are
    /// prepared again (and rebuilt if their compile settings changed) the next time an instance of that type starts.
    /// Returns the instance types whose executable config changed.
    pub fn apply_config_update(&mut self, config: Vec<ExecutableConfig>, always_compile: bool) -> Vec<InstanceType> {
        let mut changed = Vec::new();
        for new in &config {
            let old = self.config.iter().find(|e| e.instance_type == new.instance_type);
            if old == Some(new) {
                continue;
            }
            let compile_changed = new.compile.is_some() && old.is_none_or(|old| old.compile != new.compile);
            log::info!(
                "📝 Executable config changed for {} (compile_changed={}). Restart its instances to apply.",
                new.instance_type,
                compile_changed
            );
            *self.stale.entry(new.instance_type).or_default() |= compile_changed;
            changed.push(new.instance_type);
        }
        self.config = config;
        self.always_compile = always_compile;
        changed
    }

    pub fn get_executable(&self, instance_type: InstanceType) -> Option<&Executable> {
        self.prepared.iter().find(|e| e.instance_type == instance_type)
    }

    pub async fn compile_executable_if_required(&mut self, instance_type: InstanceType) -> anyhow::Result<&Executable> {
        let stale = self.stale.get(&instance_type).copied();
        if self.always_compile || stale.is_some() {
            let exec = self
                .config
                .iter()
                .find(|e| e.instance_type == instance_type)
                .ok_or_else(|| anyhow!("No executable config found for instance type '{}'", instance_type))?;
            let executable = prepare(exec, self.always_compile || stale == Some(true)).await?;
            self.stale.remove(&instance_type);
            match self.prepared.iter_mut().find(|e| e.instance_type == instance_type) {
                Some(prepared) => *prepared = executable,
                None => self.prepared.push(executable),
            }
        }

        self.get_executable(instance_type)
            .ok_or_else(|| anyhow!("No executable found for instance type '{}'", instance_type))
    }
}

/// Resolves the executable for `exec`, building it with cargo if `force_compile` is set or the binary does not exist.
async fn prepare(exec: &ExecutableConfig, force_compile: bool) -> anyhow::Result<Executable> {
    let mut exec_path = exec.get_executable_path().ok_or_else(|| {
        anyhow!(
            "No executable path or compile config found for instance type '{}'",
            exec.instance_type
        )
    })?;

    if !force_compile && exec_path.exists() {
        if exec_path.is_relative() {
            exec_path = env::current_dir()
                .context("Failed to get current directory")?
                .join(&exec_path);
        }
        return Ok(Executable {
            instance_type: exec.instance_type,
            path: exec_path,
        });
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
        compile.working_dir(),
        &compile.package_name,
        &compile.features,
        &compile.envs,
    )?;
    let status = child.wait().await?;
    if !status.success() {
        return Err(anyhow!("Failed to compile {:?}", exec.instance_type));
    }

    log::info!("🟢 Successfully compiled {}", exec.instance_type);

    let bin_path = compile.bin_path();
    Ok(Executable {
        instance_type: exec.instance_type,
        path: bin_path
            .canonicalize()
            .with_context(|| anyhow!("The compiled binary at path '{}' does not exist.", bin_path.display()))?,
    })
}

fn cargo_build<'env, P, TENVS, K, V>(
    working_dir: P,
    package: &str,
    features: &[String],
    envs: TENVS,
) -> io::Result<Child>
where
    P: AsRef<Path>,
    TENVS: IntoIterator<Item = &'env (K, V)>,
    K: AsRef<str> + 'env,
    V: AsRef<str> + 'env,
{
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--bin", package]);
    for feature in features {
        cmd.arg("--features");
        cmd.arg(feature);
    }
    cmd
        // Ensure host environment vars are available
        .envs(env::vars())
        .envs(envs.into_iter().map(|(k, v)| (k.as_ref(), v.as_ref())))
        .current_dir(working_dir)
        .kill_on_drop(true)
        .spawn()
}

pub struct Executables<'a> {
    executables: &'a [Executable],
}

impl Executables<'_> {
    pub fn get(&self, instance_type: InstanceType) -> Option<&Executable> {
        self.executables.iter().find(|e| e.instance_type == instance_type)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::CompileConfig;

    fn exec_at(instance_type: InstanceType, path: PathBuf) -> ExecutableConfig {
        ExecutableConfig {
            instance_type,
            execuable_path: Some(path),
            compile: None,
        }
    }

    #[tokio::test]
    async fn a_reloaded_executable_is_prepared_again_on_next_start() {
        let old_path = env::current_exe().unwrap();
        let new_path = old_path.parent().unwrap().to_path_buf();
        let mut manager = ExecutableManager::new(
            vec![
                exec_at(InstanceType::TariValidatorNode, old_path.clone()),
                exec_at(InstanceType::TariIndexer, old_path.clone()),
            ],
            false,
        );
        manager.prepare_all().await.unwrap();

        let changed = manager.apply_config_update(
            vec![
                exec_at(InstanceType::TariValidatorNode, new_path.clone()),
                exec_at(InstanceType::TariIndexer, old_path.clone()),
            ],
            false,
        );
        assert_eq!(changed, vec![InstanceType::TariValidatorNode]);
        assert_eq!(manager.stale.get(&InstanceType::TariValidatorNode), Some(&false));
        assert_eq!(
            manager.get_executable(InstanceType::TariValidatorNode).unwrap().path,
            old_path
        );

        let executable = manager
            .compile_executable_if_required(InstanceType::TariValidatorNode)
            .await
            .unwrap();
        assert_eq!(executable.path, new_path);
        assert!(manager.stale.is_empty());
    }

    #[test]
    fn a_compile_settings_change_forces_a_rebuild() {
        let compile = CompileConfig {
            working_dir: None,
            package_name: "tari_validator_node".to_string(),
            target_dir: None,
            features: vec![],
            envs: vec![],
        };
        let exec = ExecutableConfig {
            instance_type: InstanceType::TariValidatorNode,
            execuable_path: None,
            compile: Some(compile.clone()),
        };
        let mut manager = ExecutableManager::new(vec![exec.clone()], false);

        let mut rebuilt = exec;
        rebuilt.compile = Some(CompileConfig {
            features: vec!["mimalloc".to_string()],
            ..compile
        });
        let changed = manager.apply_config_update(vec![rebuilt], false);
        assert_eq!(changed, vec![InstanceType::TariValidatorNode]);
        assert_eq!(manager.stale.get(&InstanceType::TariValidatorNode), Some(&true));
    }
}
