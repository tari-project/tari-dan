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
    let url = format!("http://{}", address.to_string());
    let client = reqwest::Client::new();
    let body = format!(
        "{{\"method\":\"{}\", \"jsonrpc\":\"2.0\", \"id\": 1, \"params\":{}}}",
        method, params
    );
    println!("Body {:?}", body);
    let mut builder = client.post(url).header(CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        println!("With token {}", token);
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let resp = builder
        .body(body)
        .send()
        .await
        .map_err(|e| e)?
        .json::<JsonRpcResponse>()
        .await?;
    println!("Resp {:?}", resp);
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
    token: String,
    address: SocketAddr,
    signaling_server_address: SocketAddr,
    shutdown_signal: ShutdownSignal,
) -> Result<()> {
    let api = APIBuilder::new().build();

    let pc = api.new_peer_connection(get_rtc_configuration()).await?;
    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        Box::pin(async move {
            let d_on_message = d.clone();
            d.on_message(Box::new(move |msg: DataChannelMessage| {
                let d_on_message = d_on_message.clone();
                Box::pin(async move {
                    let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
                    let request = serde_json::from_str::<Request>(&msg_str).unwrap();
                    let result = handle_data(address, None, request.method, request.params)
                        .await
                        .unwrap();
                    let response = Response {
                        payload: result.to_string(),
                        id: request.id,
                    };
                    d_on_message
                        .send_text(serde_json::to_string(&response).unwrap())
                        .await
                        .unwrap();
                })
            }))
        })
    }));

    let token_clone = token.clone();
    pc.on_ice_candidate(Box::new(move |ice_candidate: Option<RTCIceCandidate>| {
        if let Some(ice_candidate) = ice_candidate {
            let token = token_clone.clone();
            println!("Ice candidate {:?}", ice_candidate);
            println!("Ice candidate {:?}", ice_candidate.to_json());
            tokio::task::spawn(async move {
                handle_data(
                    signaling_server_address,
                    Some(token),
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
        Some(token.clone()),
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
        Some(token.clone()),
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
    println!("Answer {:?}", answer);
    pc.set_local_description(answer.clone()).await?;
    handle_data(
        signaling_server_address,
        Some(token),
        "add.answer".to_string(),
        serde_json::to_string(&answer.sdp).unwrap(),
    )
    .await
    .unwrap();
    shutdown_signal.await;
    // pc.close().await?;
    Ok(())
}
