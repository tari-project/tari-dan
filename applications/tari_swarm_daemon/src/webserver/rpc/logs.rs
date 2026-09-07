//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs::DirEntry,
    io,
    path::{Path, PathBuf},
};

use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use serde::{Deserialize, Serialize};

use crate::{
    config::InstanceType,
    logfile::{self, MAX_CHUNK_BYTES},
    webserver::context::HandlerContext,
};

#[derive(Debug, Clone, Deserialize)]
pub struct ListLogFilesRequest {
    pub instance_type: InstanceType,
    #[serde(default)]
    pub index: Option<usize>,
}

/// (full path, name, path without extension)
pub type ListValidatorNodesResponse = Vec<(String, String, String)>;

pub async fn list_log_files(
    context: &HandlerContext,
    req: ListLogFilesRequest,
) -> Result<ListValidatorNodesResponse, anyhow::Error> {
    let instances = context
        .process_manager()
        .list_instances(Some(req.instance_type))
        .await?;

    let mut log_files = Vec::new();
    if let Some(index) = req.index {
        let instance = instances.get(index).ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                format!("Invalid index {index}"),
                serde_json::Value::Null,
            )
        })?;
        visit_dirs(&instance.base_path.join("log"), &mut |dir| {
            collect_current_log(dir, &instance.name, &mut log_files);
        })?;
    } else {
        for instance in instances {
            visit_dirs(&instance.base_path.join("log"), &mut |dir| {
                collect_current_log(dir, &instance.name, &mut log_files);
            })?;
        }
    }

    Ok(log_files)
}

fn collect_current_log(entry: &DirEntry, instance_name: &str, log_files: &mut ListValidatorNodesResponse) {
    let path = entry.path();
    if path.extension() != Some("log".as_ref()) || is_rotated(&path) {
        return;
    }
    let path_without_ext = path.with_extension("");
    log_files.push((
        path.to_string_lossy().to_string(),
        instance_name.to_string(),
        path_without_ext.to_string_lossy().to_string(),
    ));
}

/// True for a rotated sibling such as `ootle.2.log`, which only clutters the log list next to its live `ootle.log`.
fn is_rotated(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| Path::new(stem).extension())
        .is_some_and(|rotation| rotation.to_string_lossy().chars().all(|c| c.is_ascii_digit()))
}

fn visit_dirs<F: FnMut(&DirEntry)>(dir: &Path, cb: &mut F) -> io::Result<()> {
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, cb)?;
            } else {
                cb(&entry);
            }
        }
    }
    Ok(())
}

pub type ListStdoutLogsRequest = ListLogFilesRequest;

/// (full path, name)
pub type ListStdoutLogsResponse = Vec<(String, &'static str)>;
pub async fn list_stdout_files(
    context: &HandlerContext,
    req: ListStdoutLogsRequest,
) -> Result<ListStdoutLogsResponse, anyhow::Error> {
    let instances = context
        .process_manager()
        .list_instances(Some(req.instance_type))
        .await?;

    let mut log_files = Vec::new();
    if let Some(index) = req.index {
        let instance = instances.get(index).ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                format!("Invalid index {index}"),
                serde_json::Value::Null,
            )
        })?;
        if instance.base_path.join("stdout.log").exists() {
            log_files.push((
                instance.base_path.join("stdout.log").to_string_lossy().to_string(),
                "stdout",
            ));
        }
        if instance.base_path.join("stderr.log").exists() {
            log_files.push((
                instance.base_path.join("stderr.log").to_string_lossy().to_string(),
                "stdout",
            ));
        }
    } else {
        for instance in instances {
            if instance.base_path.join("stdout.log").exists() {
                log_files.push((
                    instance.base_path.join("stdout.log").to_string_lossy().to_string(),
                    "stdout",
                ));
            }
            if instance.base_path.join("stderr.log").exists() {
                log_files.push((
                    instance.base_path.join("stderr.log").to_string_lossy().to_string(),
                    "stdout",
                ));
            }
        }
    }

    Ok(log_files)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GetLogFileRequest {
    /// The whole tail of the file, addressed by path alone.
    Path(PathBuf),
    Window {
        path: PathBuf,
        /// Offset one past the last byte to return. Omitted means the end of the file.
        #[serde(default)]
        end: Option<u64>,
        /// Largest window to return, capped at [`MAX_CHUNK_BYTES`].
        #[serde(default)]
        max_bytes: Option<u64>,
    },
}

impl GetLogFileRequest {
    fn parts(self) -> (PathBuf, Option<u64>, Option<u64>) {
        match self {
            Self::Path(path) => (path, None, None),
            Self::Window { path, end, max_bytes } => (path, end, max_bytes),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GetLogFileResponse {
    pub contents: String,
    pub start: u64,
    pub end: u64,
    pub file_size: u64,
    /// Largest window this server will return, so a client can size its own buffer.
    pub max_chunk_bytes: u64,
}

pub async fn get_log_file(
    context: &HandlerContext,
    req: GetLogFileRequest,
) -> Result<GetLogFileResponse, anyhow::Error> {
    let (path, end, max_bytes) = req.parts();

    if !path.starts_with(&context.config().base_dir) || path.extension() != Some("log".as_ref()) || !path.exists() {
        return Err(JsonRpcError::new(
            JsonRpcErrorReason::InvalidParams,
            "Invalid file path".to_string(),
            serde_json::Value::Null,
        )
        .into());
    }

    let chunk = tokio::task::spawn_blocking(move || logfile::read_chunk(&path, end, max_bytes)).await??;

    Ok(GetLogFileResponse {
        contents: chunk.contents,
        start: chunk.start,
        end: chunk.end,
        file_size: chunk.file_size,
        max_chunk_bytes: MAX_CHUNK_BYTES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_path_still_requests_the_tail() {
        let req: GetLogFileRequest = serde_json::from_str(r#""/base/log/ootle.log""#).unwrap();
        assert_eq!(req.parts(), (PathBuf::from("/base/log/ootle.log"), None, None));
    }

    #[test]
    fn a_window_request_carries_its_bounds() {
        let req: GetLogFileRequest =
            serde_json::from_str(r#"{"path":"/base/log/ootle.log","end":4096,"max_bytes":1024}"#).unwrap();
        assert_eq!(
            req.parts(),
            (PathBuf::from("/base/log/ootle.log"), Some(4096), Some(1024))
        );
    }

    #[test]
    fn a_window_request_may_leave_its_bounds_null() {
        let req: GetLogFileRequest =
            serde_json::from_str(r#"{"path":"/base/log/ootle.log","end":null,"max_bytes":262144}"#).unwrap();
        assert_eq!(req.parts(), (PathBuf::from("/base/log/ootle.log"), None, Some(262144)));
    }

    #[test]
    fn rotated_siblings_are_recognised() {
        assert!(is_rotated(Path::new("/x/log/ootle.1.log")));
        assert!(is_rotated(Path::new("/x/log/consensus.12.log")));
        assert!(!is_rotated(Path::new("/x/log/ootle.log")));
        assert!(!is_rotated(Path::new("/x/log/json_rpc.log")));
        assert!(!is_rotated(Path::new("/x/log/some.name.log")));
    }
}
