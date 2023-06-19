//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::VecDeque,
    fmt::Formatter,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use axum_jrpc::{JsonRpcAnswer, JsonRpcRequest, JsonRpcResponse};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
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
pub struct Request {
    pub id: u64,
    pub method: String,
    pub params: String,
    pub token: String,
}

#[derive(Serialize, Debug)]
pub struct Response {
    pub id: u64,
    pub payload: String,
}

pub struct UserConfirmationRequest {
    pub website_name: String,
    pub req: Request,
    pub dc: Arc<RTCDataChannel>,
}

impl std::fmt::Debug for UserConfirmationRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserConfirmationRequest")
            .field("Request", &self.req)
            .finish()
    }
}

pub async fn make_request<T: Serialize>(
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

pub fn on_ice_candidate(
    ice_candidate: Option<RTCIceCandidate>,
    signaling_server_token_clone: String,
    signaling_server_address: SocketAddr,
) -> Pin<Box<impl futures::Future<Output = ()>>> {
    if let Some(ice_candidate) = ice_candidate {
        tokio::task::spawn(async move {
            match &ice_candidate.to_json() {
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
    Box::pin(async {})
}

pub fn on_message(
    msg: DataChannelMessage,
    d_on_message: Arc<RTCDataChannel>,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
    permissions_token: String,
    _address: SocketAddr,
    website_name: String,
) -> Pin<Box<impl futures::Future<Output = ()>>> {
    Box::pin(async move {
        match String::from_utf8(msg.data.to_vec()) {
            Ok(msg_str) => match serde_json::from_str::<Request>(&msg_str) {
                Ok(request) => {
                    let response;
                    if request.method == "get.token" {
                        match serde_json::to_string(&permissions_token) {
                            Ok(token) => {
                                response = Response {
                                    payload: token,
                                    id: request.id,
                                }
                            },
                            Err(e) => {
                                log::error!(target: LOG_TARGET, "{}", e.to_string());
                                return;
                            },
                        }
                        let text = match serde_json::to_string(&response) {
                            Ok(response) => response,
                            Err(e) => e.to_string(),
                        };
                        if let Err(e) = d_on_message.send_text(text).await {
                            log::error!(target: LOG_TARGET, "{}", e.to_string())
                        };
                    } else {
                        let mut queue = message_queue.lock().unwrap();
                        queue.push_back(UserConfirmationRequest {
                            website_name,
                            req: request,
                            dc: Arc::clone(&d_on_message),
                        });
                    }
                },
                Err(e) => log::error!(target: LOG_TARGET, "{}", e.to_string()),
            },
            Err(e) => log::error!(target: LOG_TARGET, "{}", e.to_string()),
        };
    })
}

pub fn on_data_channel(
    d: Arc<RTCDataChannel>,
    permissions_token: String,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
    address: SocketAddr,
    website_name: String,
) -> Pin<Box<impl futures::Future<Output = ()>>> {
    Box::pin(async move {
        let d_on_message = d.clone();
        d.on_message(Box::new(move |msg: DataChannelMessage| {
            on_message(
                msg,
                d_on_message.clone(),
                message_queue.clone(),
                permissions_token.clone(),
                address,
                website_name.clone(),
            )
        }))
    })
}

pub async fn webrtc_start_session(
    signaling_server_token: String,
    permissions_token: String,
    address: SocketAddr,
    signaling_server_address: SocketAddr,
    shutdown_signal: ShutdownSignal,
    message_queue: Arc<Mutex<VecDeque<UserConfirmationRequest>>>,
    website_name: String,
) -> Result<()> {
    let api = APIBuilder::new().build();

    let pc = api.new_peer_connection(get_rtc_configuration()).await?;
    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        on_data_channel(
            d,
            permissions_token.clone(),
            message_queue.clone(),
            address,
            website_name.clone(),
        )
    }));

    let signaling_server_token_clone = signaling_server_token.clone();
    pc.on_ice_candidate(Box::new(move |ice_candidate: Option<RTCIceCandidate>| {
        on_ice_candidate(
            ice_candidate,
            signaling_server_token_clone.clone(),
            signaling_server_address,
        )
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

    let ices: Vec<RTCIceCandidateInit> = serde_json::from_str(
        ices.as_str()
            .ok_or_else(|| anyhow::anyhow!("RTC Ice candidate error"))?,
    )?;
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
