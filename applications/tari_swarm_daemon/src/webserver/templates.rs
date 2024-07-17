//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io, sync::Arc};

use axum::{
    extract::{multipart::MultipartError, Multipart},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use log::{error, info};
use tari_crypto::tari_utilities::hex;
use tari_dan_engine::wasm::WasmModule;
use tari_engine_types::calculate_template_binary_hash;
use tokio::{fs, io::AsyncWriteExt};
use url::Url;

use crate::{process_manager::TemplateData, webserver::context::HandlerContext};

pub async fn upload(
    Extension(context): Extension<Arc<HandlerContext>>,
    mut value: Multipart,
) -> Result<(), UploadError> {
    let Some(field) = value.next_field().await? else {
        error!("🌐 Upload template: no field found");
        // TODO: return error
        return Ok(());
    };

    let name = field.file_name().unwrap_or("unnamed-template").to_string();
    let bytes = field.bytes().await?;
    let hash = calculate_template_binary_hash(&bytes);
    let dest_file = format!("{}-{}.wasm", slug(&name), hex::to_hex(hash.as_ref()));
    let dest_path = context.config().base_dir.join("templates").join(&dest_file);

    // Load the struct name from the wasm.
    let loaded = WasmModule::load_template_from_code(&bytes).map_err(|e| UploadError::Other(e.into()))?;
    let name = loaded.template_def().template_name().to_string();
    let mut file = fs::File::create(dest_path).await?;
    file.write_all(&bytes).await?;
    info!("🌐 Upload template {} bytes", bytes.len());

    let data = TemplateData {
        name,
        version: 0,
        contents_hash: hash,
        contents_url: Url::parse(&format!(
            "http://localhost:{}/templates/{}",
            context.config().webserver.bind_address.port(),
            dest_file
        ))
        .unwrap(),
    };

    match context.process_manager().register_template(data).await {
        Ok(()) => {
            info!("🌐 Registered template");
            Ok(())
        },
        Err(err) => {
            error!("🌐 Registering template failed: {}", err);
            Err(err.into())
        },
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error(transparent)]
    MultiPartError(#[from] MultipartError),
    #[error(transparent)]
    Other(anyhow::Error),
}

impl From<io::Error> for UploadError {
    fn from(value: io::Error) -> Self {
        UploadError::Other(value.into())
    }
}

impl From<anyhow::Error> for UploadError {
    fn from(value: anyhow::Error) -> Self {
        UploadError::Other(value)
    }
}

impl IntoResponse for UploadError {
    fn into_response(self) -> axum::response::Response {
        match self {
            UploadError::MultiPartError(err) => err.into_response(),
            UploadError::Other(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Upload error: {}", err)).into_response()
            },
        }
    }
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' => c,
            '0'..='9' => c,
            _ => '-',
        })
        .collect()
}
