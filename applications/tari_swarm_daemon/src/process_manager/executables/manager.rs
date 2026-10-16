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
    /// The config each prepared executable was resolved from.
    prepared_from: HashMap<InstanceType, ExecutableConfig>,
}

impl ExecutableManager {
    pub fn new(config: Vec<ExecutableConfig>, always_compile: bool) -> Self {
        Self {
            config,
            always_compile,
            prepared: Vec::new(),
            prepared_from: HashMap::new(),
        }
    }

    pub async fn prepare_all(&mut self) -> anyhow::Result<Executables<'_>> {
        log::info!("Compiling {} executables", self.config.len());
        self.prepared.clear();
        self.prepared_from.clear();

        let always_compile = self.always_compile;
        let mut tasks = self
            .config
            .iter()
            .filter(|exec| exec.get_executable_path().is_some())
            .map(|exec| async move { prepare(exec, always_compile).await.map(|e| (e, exec)) })
            .collect::<FuturesUnordered<_>>();

        while let Some(result) = tasks.next().await {
            let (executable, exec) = result?;
            self.prepared_from.insert(exec.instance_type, exec.clone());
            self.prepared.push(executable);
        }

        Ok(Executables {
            executables: &self.prepared,
        })
    }

    /// Replaces the executable config with one reloaded from the config file. Returns the instance types whose
    /// prepared executable no longer matches the config; they are prepared again the next time an instance of that
    /// type starts.
    pub fn apply_config_update(&mut self, config: Vec<ExecutableConfig>, always_compile: bool) -> Vec<InstanceType> {
        self.config = config;
        self.always_compile = always_compile;

        let changed = self
            .prepared_from
            .iter()
            .filter(|(instance_type, prepared_from)| self.find_config(**instance_type) != Some(*prepared_from))
            .map(|(instance_type, _)| *instance_type)
            .collect::<Vec<_>>();
        for instance_type in &changed {
            log::info!("📝 Executable config changed for {instance_type}. Restart its instances to apply.");
        }
        changed
    }

    pub fn get_executable(&self, instance_type: InstanceType) -> Option<&Executable> {
        self.prepared.iter().find(|e| e.instance_type == instance_type)
    }

    pub async fn compile_executable_if_required(&mut self, instance_type: InstanceType) -> anyhow::Result<&Executable> {
        if let Some(force_compile) = self.reprepare(instance_type) {
            let exec = self
                .find_config(instance_type)
                .cloned()
                .ok_or_else(|| anyhow!("No executable config found for instance type '{}'", instance_type))?;
            let executable = prepare(&exec, force_compile).await?;
            self.prepared_from.insert(instance_type, exec);
            match self.prepared.iter_mut().find(|e| e.instance_type == instance_type) {
                Some(prepared) => *prepared = executable,
                None => self.prepared.push(executable),
            }
        }

        self.get_executable(instance_type)
            .ok_or_else(|| anyhow!("No executable found for instance type '{}'", instance_type))
    }

    /// Returns `Some(force_compile)` if the executable for `instance_type` must be prepared again before an instance
    /// of that type starts: always under `always_compile`, otherwise when the config differs from the one it was
    /// prepared from. A changed compile config forces a rebuild even if the binary exists.
    fn reprepare(&self, instance_type: InstanceType) -> Option<bool> {
        let current = self.find_config(instance_type);
        let prepared_from = self.prepared_from.get(&instance_type);
        if !self.always_compile && current == prepared_from {
            return None;
        }
        let compile_changed = current.is_some_and(|current| {
            current.compile.is_some() &&
                prepared_from.is_some_and(|prepared_from| prepared_from.compile != current.compile)
        });
        Some(self.always_compile || compile_changed)
    }

    fn find_config(&self, instance_type: InstanceType) -> Option<&ExecutableConfig> {
        self.config.iter().find(|e| e.instance_type == instance_type)
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

    fn exec_at(path: PathBuf) -> ExecutableConfig {
        ExecutableConfig {
            instance_type: InstanceType::TariValidatorNode,
            execuable_path: Some(path),
            compile: None,
        }
    }

    fn with_features(mut exec: ExecutableConfig, features: &[&str]) -> ExecutableConfig {
        exec.compile = Some(CompileConfig {
            working_dir: None,
            package_name: "tari_validator_node".to_string(),
            target_dir: None,
            features: features.iter().map(|f| f.to_string()).collect(),
            envs: vec![],
        });
        exec
    }

    async fn prepared_manager(exec: ExecutableConfig) -> ExecutableManager {
        let mut manager = ExecutableManager::new(vec![exec], false);
        manager.prepare_all().await.unwrap();
        manager
    }

    #[tokio::test]
    async fn a_reloaded_executable_is_prepared_again_on_next_start() {
        let old_path = env::current_exe().unwrap();
        let new_path = old_path.parent().unwrap().to_path_buf();
        let mut manager = prepared_manager(exec_at(old_path.clone())).await;

        let changed = manager.apply_config_update(vec![exec_at(new_path.clone())], false);
        assert_eq!(changed, vec![InstanceType::TariValidatorNode]);
        assert_eq!(manager.reprepare(InstanceType::TariValidatorNode), Some(false));
        assert_eq!(
            manager.get_executable(InstanceType::TariValidatorNode).unwrap().path,
            old_path
        );

        let executable = manager
            .compile_executable_if_required(InstanceType::TariValidatorNode)
            .await
            .unwrap();
        assert_eq!(executable.path, new_path);
        assert_eq!(manager.reprepare(InstanceType::TariValidatorNode), None);
        assert!(manager.apply_config_update(vec![exec_at(new_path)], false).is_empty());
    }

    #[tokio::test]
    async fn a_compile_settings_change_forces_a_rebuild() {
        let exec = exec_at(env::current_exe().unwrap());
        let mut manager = prepared_manager(with_features(exec.clone(), &[])).await;

        manager.apply_config_update(vec![with_features(exec, &["mimalloc"])], false);
        assert_eq!(manager.reprepare(InstanceType::TariValidatorNode), Some(true));
    }

    #[tokio::test]
    async fn only_the_latest_reload_decides_how_an_executable_is_prepared() {
        let old_path = env::current_exe().unwrap();
        let exec = exec_at(old_path.clone());
        let mut manager = prepared_manager(with_features(exec.clone(), &[])).await;

        manager.apply_config_update(vec![with_features(exec.clone(), &["mimalloc"])], false);
        let changed = manager.apply_config_update(vec![with_features(exec.clone(), &[])], false);
        assert!(changed.is_empty());
        assert_eq!(manager.reprepare(InstanceType::TariValidatorNode), None);

        let new_path = old_path.parent().unwrap().to_path_buf();
        manager.apply_config_update(vec![with_features(exec, &["mimalloc"])], false);
        manager.apply_config_update(vec![exec_at(new_path.clone())], false);
        assert_eq!(manager.reprepare(InstanceType::TariValidatorNode), Some(false));
        let executable = manager
            .compile_executable_if_required(InstanceType::TariValidatorNode)
            .await
            .unwrap();
        assert_eq!(executable.path, new_path);
    }

    #[tokio::test]
    async fn a_removed_executable_is_reported_as_changed() {
        let mut manager = prepared_manager(exec_at(env::current_exe().unwrap())).await;

        let changed = manager.apply_config_update(vec![], false);
        assert_eq!(changed, vec![InstanceType::TariValidatorNode]);
        assert!(
            manager
                .compile_executable_if_required(InstanceType::TariValidatorNode)
                .await
                .is_err()
        );
    }
}
