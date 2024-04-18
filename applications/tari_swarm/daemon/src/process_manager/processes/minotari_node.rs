//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::{anyhow, Context};
use tokio::{fs::File, io::AsyncReadExt};

use crate::process_manager::Instance;

pub struct MinoTariNodeProcess {
    instance: Instance,
}

impl MinoTariNodeProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    // pub async fn connect_client(&self) -> anyhow::Result<BaseNodeGrpcClient<tonic::transport::Channel>> {
    //     let port = self
    //         .instance
    //         .allocated_ports()
    //         .get("grpc")
    //         .ok_or_else(|| anyhow!("No grpc port allocated"))?;
    //     let client = BaseNodeGrpcClient::connect(format!("http://localhost:{}", port)).await?;
    //     Ok(client)
    // }

    pub async fn get_identity(&self) -> anyhow::Result<String> {
        // We cannot call identify because we'd need to override the allowed methods via cli, and this is not
        // supported. So we read from the base node identity file
        let mut config = File::open(self.instance.base_path().join("config").join("base_node_id.json"))
            .await
            .context("Loading base node ID failed")?;
        let mut s = String::new();
        config.read_to_string(&mut s).await?;
        let identity = json5::from_str::<serde_json::Value>(&s)?;
        let public_key = identity["public_key"]
            .as_str()
            .ok_or_else(|| anyhow!("public_key not found or not a string"))?;
        let public_addresses = identity["public_addresses"]
            .as_array()
            .ok_or_else(|| anyhow!("public_addresses not found or not an array"))?;
        let public_addresses = public_addresses
            .iter()
            .map(|v| v.as_str().ok_or_else(|| anyhow!("public_address not a string")))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("{}::{}", public_key, public_addresses.join(",")))
    }
}
