//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum_jrpc::{JsonRpcAnswer, JsonRpcResponse};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
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

#[derive(Deserialize, Debug)]
struct Request {
    id: u64,
    method: String,
    params: String,
    token: String,
}

#[derive(Serialize, Debug)]
struct Response {
    id: u64,
    payload: String,
}

pub async fn handle_data(
    address: SocketAddr,
    token: Option<String>,
    method: String,
    params: String,
) -> Result<serde_json::Value> {
    let url = format!("http://{}", address);
    let client = reqwest::Client::new();
    let body = format!(
        "{{\"method\":\"{}\", \"jsonrpc\":\"2.0\", \"id\": 1, \"params\":{}}}",
        method, params
    );
    let mut builder = client.post(url).header(CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let resp = builder.body(body).send().await?.json::<JsonRpcResponse>().await?;
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
        let permissions_token = permissions_token.clone();
        Box::pin(async move {
            let d_on_message = d.clone();
            d.on_message(Box::new(move |msg: DataChannelMessage| {
                let d_on_message = d_on_message.clone();
                let permissions_token = permissions_token.clone();
                Box::pin(async move {
                    let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
                    let request = serde_json::from_str::<Request>(&msg_str).unwrap();
                    let response;
                    if request.method == "get.token" {
                        response = Response {
                            payload: serde_json::to_string(&permissions_token).unwrap(),
                            id: request.id,
                        };
                    } else {
                        let result = handle_data(address, Some(request.token), request.method, request.params)
                            .await
                            .unwrap();
                        response = Response {
                            payload: result.to_string(),
                            id: request.id,
                        };
                    }
                    d_on_message
                        .send_text(serde_json::to_string(&response).unwrap())
                        .await
                        .unwrap();
                })
            }))
        })
    }));

    let signaling_server_token_clone = signaling_server_token.clone();
    pc.on_ice_candidate(Box::new(move |ice_candidate: Option<RTCIceCandidate>| {
        if let Some(ice_candidate) = ice_candidate {
            let signaling_server_token = signaling_server_token_clone.clone();
            tokio::task::spawn(async move {
                handle_data(
                    signaling_server_address,
                    Some(signaling_server_token),
                    "add.answer_ice_candidate".to_string(),
                    serde_json::to_string(&ice_candidate.to_json().unwrap()).unwrap(),
                )
                .await
                .unwrap();
            });
        }
        Box::pin(async {})
    }));

    let offer = handle_data(
        signaling_server_address,
        Some(signaling_server_token.clone()),
        "get.offer".to_string(),
        serde_json::to_string("").unwrap(),
    )
    .await
    .unwrap();
    let offer: String = serde_json::from_str(offer.as_str().unwrap()).unwrap();
    let desc = RTCSessionDescription::offer(offer)?;
    pc.set_remote_description(desc).await?;

    let ices = handle_data(
        signaling_server_address,
        Some(signaling_server_token.clone()),
        "get.offer_ice_candidates".to_string(),
        serde_json::to_string("").unwrap(),
    )
    .await
    .unwrap();
    let ices: Vec<String> = serde_json::from_str(ices.as_str().unwrap()).unwrap();
    for ice_candidate in ices {
        let ice_candidate: RTCIceCandidateInit = serde_json::from_str(ice_candidate.as_str()).unwrap();
        pc.add_ice_candidate(ice_candidate).await?;
    }
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;
    handle_data(
        signaling_server_address,
        Some(signaling_server_token),
        "add.answer".to_string(),
        serde_json::to_string(&answer.sdp).unwrap(),
    )
    .await
    .unwrap();
    shutdown_signal.await;
    // pc.close().await?;
    Ok(())
}
