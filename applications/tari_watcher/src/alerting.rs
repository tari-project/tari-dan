// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::{bail, Result};
use reqwest::StatusCode;
use serde_json::json;

pub trait Alerting {
    fn new(url: String, channel_id: String, credentials: String) -> Self;

    // Sends an alert message to the service
    async fn alert(&mut self, message: &str) -> Result<()>;

    // Checks that the service is reachable
    async fn ping(&self) -> Result<()>;

    // Statistics on the alerts sent
    // todo: expand granularity and types of stats
    fn stats(&self) -> Result<u64>;
}

pub struct MatterMostNotifier {
    // Mattermost server URL
    server_url: String,
    // Mattermost channel ID used for alerts
    channel_id: String,
    // User token (retrieved after login)
    credentials: String,
    // Alerts sent since last reset
    alerts_sent: u64,
    // HTTP client
    client: reqwest::Client,
}

impl Alerting for MatterMostNotifier {
    fn new(server_url: String, channel_id: String, credentials: String) -> Self {
        Self {
            server_url,
            channel_id,
            credentials,
            alerts_sent: 0,
            client: reqwest::Client::new(),
        }
    }

    async fn alert(&mut self, message: &str) -> Result<()> {
        const LOGIN_ENDPOINT: &str = "/api/v4/posts";
        let url = format!("{}{}", self.server_url, LOGIN_ENDPOINT);
        let req = json!({
            "channel_id": self.channel_id,
            "message": message,
        });
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .header("Authorization", format!("Bearer {}", self.credentials))
            .send()
            .await?;

        if resp.status() != StatusCode::CREATED {
            bail!("Failed to send alert, got response: {}", resp.status());
        }

        self.alerts_sent += 1;

        Ok(())
    }

    async fn ping(&self) -> Result<()> {
        const PING_ENDPOINT: &str = "/api/v4/users/me";
        if self.server_url.is_empty() {
            bail!("Server URL is empty");
        } else if self.credentials.is_empty() {
            bail!("Credentials are empty");
        }

        let url = format!("{}{}", self.server_url, PING_ENDPOINT);
        let resp = self
            .client
            .get(url.clone())
            .header("Authorization", format!("Bearer {}", self.credentials))
            .send()
            .await?;

        if resp.status() != StatusCode::OK {
            bail!("Failed to ping, got response: {}", resp.status());
        }

        return Ok(());
    }

    fn stats(&self) -> Result<u64> {
        return Ok(self.alerts_sent);
    }
}
