//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum_jrpc::{JsonRpcAnswer, JsonRpcRequest, JsonRpcResponse};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json as json;
use serde_json::json;
use tari_shutdown::ShutdownSignal;
use webrtc::{
    api::APIBuilder,
    data_channel::{data_channel_message::DataChannelMessage, RTCDataChannel},
    ice_transport::{
        ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
        ice_server::RTCIceServer,
    },
    peer_connection::{configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::webrtc";

#[derive(Deserialize, Debug)]
struct Request {
    id: u64,
    method: String,
    params: json::Value,
    token: Option<String>,
}

#[derive(Serialize, Debug)]
struct Response {
    id: u64,
    payload: json::Value,
}

async fn make_request<T: Serialize>(
    address: SocketAddr,
    token: Option<String>,
    method: String,
    params: T,
) -> Result<serde_json::Value> {
    let url = format!("http://{}", address);
    let client = reqwest::Client::new();
    let body = JsonRpcRequest {
        id: 0,
        jsonrpc: "2.0".to_string(),
        method,
        params: serde_json::to_value(params)?,
    };
    let mut builder = client.post(url).header(CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let resp = builder.json(&body).send().await?.json::<JsonRpcResponse>().await?;
    match resp.result {
        JsonRpcAnswer::Result(result) => Ok(result),
        JsonRpcAnswer::Error(error) => Err(anyhow::Error::msg(error.to_string())),
    }
}

fn get_rtc_configuration() -> RTCConfiguration {
    RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub async fn on_ice_candidate(
    ice_candidate: Option<RTCIceCandidate>,
    signaling_server_token_clone: String,
    signaling_server_address: SocketAddr,
) {
    if let Some(ice_candidate) = ice_candidate {
        tokio::task::spawn(async move {
            match ice_candidate.to_json() {
                Ok(ice_candidate) => {
                    if let Err(err) = make_request(
                        signaling_server_address,
                        Some(signaling_server_token_clone),
                        "add.answer_ice_candidate".to_string(),
                        ice_candidate,
                    )
                    .await
                    {
                        log::error!(target: LOG_TARGET, "Error sending ice candidate: {}", err);
                    }
                },
                Err(e) => {
                    log::error!(target: LOG_TARGET, "Error sending ice candidate: {}", e);
                },
            };
        });
    }
}

pub async fn on_message(
    msg: DataChannelMessage,
    d_on_message: Arc<RTCDataChannel>,
    permissions_token: String,
    address: SocketAddr,
) -> anyhow::Result<()> {
    let request = serde_json::from_reader::<_, Request>(&mut msg.data.as_ref())?;

    let response;
    if request.method == "get.token" {
        let token = json::to_value(permissions_token)?;
        response = Response {
            payload: token,
            id: request.id,
        }
    } else {
        let result = make_request(address, request.token, request.method, request.params)
            .await
            .unwrap_or_else(|e| json!({"error": e.to_string()}));
        response = Response {
            payload: result,
            id: request.id,
        };
    }
    let text = serde_json::to_string(&response).unwrap_or_else(|e| e.to_string());
    d_on_message.send_text(text).await?;
    Ok(())
}

pub async fn on_data_channel(d: Arc<RTCDataChannel>, permissions_token: String, address: SocketAddr) {
    let d_on_message = d.clone();
    d.on_message(Box::new(move |msg: DataChannelMessage| {
        let d_on_message = d_on_message.clone();
        let permissions_token = permissions_token.clone();
        Box::pin(async move {
            if let Err(err) = on_message(msg, d_on_message.clone(), permissions_token.clone(), address).await {
                log::error!(target: LOG_TARGET, "Error handling message: {}", err);
            }
        })
    }))
}

pub async fn webrtc_start_session(
    signaling_server_token: String,
    permissions_token: String,
    address: SocketAddr,
    signaling_server_address: SocketAddr,
    shutdown_signal: ShutdownSignal,
) -> Result<()> {
    let api = APIBuilder::new().build();

    let pc = api.new_peer_connection(get_rtc_configuration()).await?;
    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        Box::pin(on_data_channel(d, permissions_token.clone(), address))
    }));

    let signaling_server_token_clone = signaling_server_token.clone();
    pc.on_ice_candidate(Box::new(move |ice_candidate: Option<RTCIceCandidate>| {
        Box::pin(on_ice_candidate(
            ice_candidate,
            signaling_server_token_clone.clone(),
            signaling_server_address,
        ))
    }));

    let offer = make_request(
        signaling_server_address,
        Some(signaling_server_token.clone()),
        "get.offer".to_string(),
        json!({}),
    )
    .await?;

    let desc = RTCSessionDescription::offer(
        offer
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("RTC Offer error"))?
            .to_string(),
    )?;
    pc.set_remote_description(desc).await?;

    let ices = make_request(
        signaling_server_address,
        Some(signaling_server_token.clone()),
        "get.offer_ice_candidates".to_string(),
        json!({}),
    )
    .await?;

    let ices: Vec<RTCIceCandidateInit> = serde_json::from_value(ices)?;
    for ice_candidate in ices {
        println!("ice_candidate {:?}", ice_candidate);
        // let ice_candidate: RTCIceCandidateInit = serde_json::from_str(ice_candidate.as_str())?;
        pc.add_ice_candidate(ice_candidate).await?;
    }
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    make_request(
        signaling_server_address,
        Some(signaling_server_token),
        "add.answer".to_string(),
        &answer.sdp,
    )
    .await?;
    shutdown_signal.await;
    // pc.close().await?;
    Ok(())
}
