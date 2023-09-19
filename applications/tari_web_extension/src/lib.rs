//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{Arc, Mutex};

use js_sys::{Array, Function, Reflect, JSON};
use lazy_static::lazy_static;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    MessageEvent,
    Request,
    RequestInit,
    Response,
    RtcConfiguration,
    RtcDataChannel,
    RtcIceCandidate,
    RtcIceServer,
    RtcPeerConnection,
    RtcPeerConnectionIceEvent,
    RtcSdpType,
    RtcSessionDescriptionInit,
};

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

macro_rules! console_warn {
    ($($t:tt)*) => (warn(&format_args!($($t)*).to_string()))
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
    #[wasm_bindgen(js_namespace = console)]
    fn warn(s: &str);
}

// Setup `lol_alloc` as the global allocator for wasm32 targets with the "lol_alloc" feature is enabled.
#[cfg(all(feature = "lol_alloc", target_arch = "wasm32"))]
#[global_allocator]
static ALLOC: lol_alloc::LockedAllocator<lol_alloc::FreeListAllocator> =
    lol_alloc::LockedAllocator::new(lol_alloc::FreeListAllocator::new());

lazy_static! {
    static ref ICE_CNT: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    static ref SESSION: Arc<Mutex<String>> = Arc::new(Mutex::new("".to_string()));
}

async fn save_offer(session: &String, offer: &str) -> Result<(), JsValue> {
    let url = format!("https://webrtc-test-d081d-default-rtdb.europe-west1.firebasedatabase.app/{session}/offer.json");
    let body = format!("{{\"offer\": \"{}\"}}", offer.escape_default());

    let mut opts = RequestInit::new();
    opts.method("PUT");
    opts.mode(web_sys::RequestMode::Cors);
    // opts.headers(&JsValue::from_str("Content-Type: application/json"));
    console_log!("{:?}", body);
    opts.body(Some(&JsValue::from_str(body.as_str())));
    let request = Request::new_with_str_and_init(&url, &opts)?;
    let window = web_sys::window().ok_or("no global `window` exists")?;
    JsFuture::from(window.fetch_with_request(&request)).await?;
    Ok(())
}

fn save_ice_candidate(session: &String, ice: &RtcIceCandidate) -> Result<(), JsValue> {
    let url;
    {
        let mut id = ICE_CNT.lock().unwrap();
        url = format!(
            "https://webrtc-test-d081d-default-rtdb.europe-west1.firebasedatabase.app/{session}/offer_ice/{}.json",
            *id
        );
        *id += 1;
    }

    let mut opts = RequestInit::new();
    opts.method("PUT");
    opts.mode(web_sys::RequestMode::Cors);
    // opts.headers(&JsValue::from_str("Content-Type: application/json"));
    opts.body(Some(&JsValue::from_str(
        format!("{}", JSON::stringify(&JsValue::from(ice.to_json())).unwrap()).as_str(),
    )));
    let request = Request::new_with_str_and_init(&url, &opts)?;
    let window = web_sys::window().ok_or("no global `window` exists")?;
    console_log!("saving ice");
    #[allow(unused_must_use)]
    {
        // TODO: somehow solve the async call in sync closure, fortunately this is JsFuture and not rust. So it will
        // run, we just don't wait for it.
        JsFuture::from(window.fetch_with_request(&request));
    }
    Ok(())
}

#[wasm_bindgen(js_name = "getPeerConnection")]
pub async fn get_peer_connection() -> Result<JsValue, JsValue> {
    let session = get_session_id().as_string().unwrap();
    console_log!("Session id : {}", session);
    let mut config = RtcConfiguration::new();
    let ice_servers = Array::new();
    let mut ice_server = RtcIceServer::new();
    ice_server.url("stun:stun.l.google.com:19302");
    ice_server.credential_type(web_sys::RtcIceCredentialType::Password);
    ice_servers.push(&JsValue::from(ice_server));
    config.ice_servers(&JsValue::from(&ice_servers));
    let pc = RtcPeerConnection::new_with_configuration(&config)?;
    Ok(JsValue::from(pc))
}

#[wasm_bindgen(js_name = "getDataChannel")]
pub async fn get_data_channel(pc: RtcPeerConnection, callback: JsValue) -> Result<JsValue, JsValue> {
    let session = SESSION.lock().unwrap().clone();
    let dc = pc.create_data_channel("my-data");
    let function = callback
        .dyn_into::<Function>()
        .map_err(|_| JsError::new("The provided callback is not a function!"))?;
    let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |ev: MessageEvent| {
        if let Some(message) = ev.data().as_string() {
            console_warn!("{:?}", message);
            function
                .call1(&JsValue::undefined(), &JsValue::from_str(message.as_str()))
                .unwrap();
        }
    });
    dc.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
    onmessage_callback.forget();

    let onopen_callback = Closure::<dyn FnMut()>::new(move || {
        console_log!("Data channel is open!");
    });
    dc.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
    onopen_callback.forget();
    let session_clone = session.clone();
    let onicecandidate_callback = Closure::<dyn FnMut(_)>::new(move |ev: RtcPeerConnectionIceEvent| {
        if let Some(candidate) = ev.candidate() {
            save_ice_candidate(&session_clone, &candidate).unwrap();
        }
    });
    pc.set_onicecandidate(Some(onicecandidate_callback.as_ref().unchecked_ref()));
    onicecandidate_callback.forget();

    let offer = JsFuture::from(pc.create_offer()).await?;
    let offer_sdp = Reflect::get(&offer, &JsValue::from_str("sdp"))?.as_string().unwrap();
    save_offer(&session, &offer_sdp).await?;
    let mut offer_obj = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
    offer_obj.sdp(&offer_sdp);
    JsFuture::from(pc.set_local_description(&offer_obj)).await?;

    Ok(JsValue::from(dc))
}

#[wasm_bindgen(js_name = "sendMessage")]
pub async fn send_message(dc: RtcDataChannel, msg: String) -> Result<(), JsValue> {
    console_log!("{:?}", msg);
    dc.send_with_str(msg.as_str())
}

#[wasm_bindgen(js_name = "getSessionId")]
pub fn get_session_id() -> JsValue {
    if (*SESSION.lock().unwrap()).is_empty() {
        *SESSION.lock().unwrap() = format!("{}", (js_sys::Math::random() * ((-1i64) as u64) as f64) as u64);
    }
    JsValue::from_str(SESSION.lock().unwrap().as_str())
}

#[wasm_bindgen(js_name = "setAnswer")]
pub async fn set_answer(pc: RtcPeerConnection) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no global `window` exists").unwrap();

    let session = SESSION.lock().unwrap().clone();
    let url =
        format!("https://webrtc-test-d081d-default-rtdb.europe-west1.firebasedatabase.app/{session}/answer_ice.json",);
    let mut opts = RequestInit::new();
    opts.method("GET");
    opts.mode(web_sys::RequestMode::Cors);
    let request = Request::new_with_str_and_init(&url, &opts).unwrap();
    let response = JsFuture::from(window.fetch_with_request(&request)).await.unwrap();
    let resp: Response = response.dyn_into().unwrap();
    let json = JsFuture::from(resp.json()?).await?;
    if json != JsValue::null() {
        let json: serde_json::Value =
            serde_json::from_str(&JSON::stringify(&json).unwrap().as_string().unwrap()).unwrap();
        let ices = json.as_array().unwrap();

        for ice in ices {
            let ic = RtcIceCandidate::from(serde_wasm_bindgen::to_value(ice).unwrap());
            JsFuture::from(pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&ic)))
                .await
                .unwrap();
        }
    }

    let url =
        format!("https://webrtc-test-d081d-default-rtdb.europe-west1.firebasedatabase.app/{session}/answer.json",);

    let request = Request::new_with_str_and_init(&url, &opts).unwrap();

    let response = JsFuture::from(window.fetch_with_request(&request)).await.unwrap();
    let resp: Response = response.dyn_into().unwrap();
    let json = JsFuture::from(resp.json()?).await?;
    if json != JsValue::null() {
        let json: serde_json::Value =
            serde_json::from_str(&JSON::stringify(&json).unwrap().as_string().unwrap()).unwrap();
        //         console_log!("resp {:?}", json.get("offer"));
        let mut answer_obj = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        let answer_sdp = json.get("answer").unwrap();
        answer_obj.sdp(answer_sdp.as_str().unwrap());
        console_log!("{:?}", answer_obj);
        JsFuture::from(pc.set_remote_description(&answer_obj)).await?;
    }
    Ok(())
}
