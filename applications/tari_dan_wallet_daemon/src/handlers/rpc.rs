//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::handlers::HandlerContext;

pub async fn handle_discover(
    _context: &HandlerContext,
    _value: serde_json::Value,
) -> Result<serde_json::Value, anyhow::Error> {
    Ok(serde_json::from_str(include_str!("../../openrpc.json"))?)
}

pub async fn handle_init(
    context: &HandlerContext,
    _value: serde_json::Value,
) -> Result<serde_json::Value, anyhow::Error> {
    let token = context.jwt().generate()?;
    Ok(serde_json::to_value(token)?)
}
