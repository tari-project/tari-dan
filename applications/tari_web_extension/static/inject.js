//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

var wasm_url = document.currentScript.getAttribute('wasm');

class SignaligServer {
  constructor() {
    this.token = undefined;
  }

  async initToken() {
    this.token = await this.#getToken()
    window.tari_token = this.token;
  }

  async #jsonRpc(method, token, params) {
    let id = 0;
    id += 1;
    let address = 'localhost:9100';
    let text = await (await fetch('json_rpc_address')).text();
    if (/^\d+(\.\d+){3}:[0-9]+$/.test(text)) {
      address = text;
    }
    let headers = { 'Content-Type': 'application/json' };
    if (token) {
      headers["Authorization"] = `Bearer ${token}`;
    }
    if (!params) {
      params = []
    }
    let response = await fetch(`http://${address}`, {
      method: 'POST',
      body: JSON.stringify({
        method: method,
        jsonrpc: '2.0',
        id: id,
        params: params,
      }),
      headers: headers
    });
    let json = await response.json();
    if (json.error) {
      throw json.error;
    }
    return json.result;
  }

  async #getToken() {
    return await this.#jsonRpc("get.jwt");
  }

  async storeIceCandidate(ice_candidate) {
    return await this.#jsonRpc("add.offer_ice_candidate", this.token, ice_candidate)
  }

  async storeOffer(offer) {
    return await this.#jsonRpc("add.offer", this.token, offer.sdp)
  }

  async getAnswer() {
    return await this.#jsonRpc("get.answer", this.token)
  }

  async getIceCandidates() {
    return await this.#jsonRpc("get.answer_ice_candidates", this.token)
  }
}
// In webrtc one end is the offer and the other one is the answer, based on how we want to connect, 
// the web extension will always be the offer.
class WebRtc {
  constructor() {
    this.peerConnection = new RTCPeerConnection(this.config());
    this.dataChannel = this.peerConnection.createDataChannel("my-data");
    this.signalingServer = new SignaligServer();
    this.messageId = 0;
    this.lock = Promise.resolve();
    this.callbacks = {};
  }


  async init() {
    await this.signalingServer.initToken();
    // Setup our receiving end
    this.dataChannel.onmessage = (message) => {
      console.log("OnMessage", message);
      let response = JSON.parse(message.data)
      // The response should contain id, to identify the Promise.resolve, that is waiting for this result
      let [resolve, reject] = this.callbacks[response.id];
      delete this.callbacks[response.id];
      try {
        resolve(JSON.parse(response.payload));
      }
      catch {
        reject(response.payload);
      }
    };
    this.dataChannel.onopen = () => {
      // This is currently just a user notification, but we can use the pc signaling state to know if it is open.
      console.log("Data channel is open!");
    };
    this.peerConnection.onicecandidate = (event) => {
      if (event?.candidate) {
        // Store the ice candidates, so the other end can add them
        this.signalingServer.storeIceCandidate(event.candidate).then((resp) => {
          console.log("Candidate stored", resp);
        })
      }
    };
    // Create offer
    this.offer = await this.peerConnection.createOffer();
    // Set the offer as our local sdp, at this point it will start getting the ice candidates
    this.peerConnection.setLocalDescription(this.offer);
    // Store the offer so the other end can set it as a remote sdp
    this.signalingServer.storeOffer(this.offer).then((resp) => {
      console.log("Offer stored", resp);
    })
  }

  async setAnswer() {
    // This is called once the other end got the offer and ices and created and store an answer and its ice candidates
    // We get its answer sdp
    let sdp = JSON.parse((await this.signalingServer.getAnswer()));
    // And its ice candidates
    let iceCandidates = await this.signalingServer.getIceCandidates();
    // For us the answer is remote sdp
    let answer = new RTCSessionDescription({ sdp, type: "answer" });
    this.peerConnection.setRemoteDescription(answer);
    // We add all the ice candidates to connect, the other end is doing the same with our ice candidates
    iceCandidates = JSON.parse(iceCandidates);
    for (const iceCandidate of iceCandidates) {
      this.peerConnection.addIceCandidate(JSON.parse(iceCandidate));
    }
  }

  async getNextMessageId() {
    // Javascript "Mutex" :-)
    // We need to make sure the ids are unique so we can assign the result to the correct promises.
    await this.lock;
    let messageId = this.messageId;
    this.messageId += 1;
    this.lock = Promise.resolve();
    return messageId;
  }

  async sendMessage(method, ...args) {
    var timeout = 0;
    if (args.length > 0) {
      console.log(args.length)
      if (args[args.length - 1]?.timeout) {
        timeout = args.pop().timeout;
      }
    }
    console.log(args, 'timeout', timeout);
    // Generate a unique id
    let messageId = await this.getNextMessageId();
    return new Promise((resolve, reject) => {
      // We store the resolve callback for this request, 
      // so once the data channel receives a response we know where to return the data
      this.callbacks[messageId] = [resolve, reject];
      if (timeout > 0) {
        setTimeout(() => {
          delete this.callbacks[messageId];
          reject(new Error("Timeout"));
        }, timeout)
      }
      // Make the actual call to the wallet daemon
      this.dataChannel.send(JSON.stringify({ id: messageId, method, params: JSON.stringify(args) }));
    });
  }

  config() {
    return { iceServers: [{ urls: "stun:stun.l.google.com:19302" }] };
  }
}

async function run() {
  window.tari = new WebRtc();
  await window.tari.init();
  const index_bg = await import("./index_bg.js");

  const importObject = {
    "./index_bg.js": index_bg
  };
  let wasm = await fetch(wasm_url);
  wasm = await wasm.arrayBuffer();
  let mod = await WebAssembly.compile(wasm);
  try {
    globalThis.wasm = (await WebAssembly.instantiate(mod, importObject)).exports
  } catch (error) {
    console.error('Instantiate error:', error)
  }
  index_bg.__wbg_set_wasm(globalThis.wasm);
}

try {
  run();
}
catch (error) {
  console.error(error)
}
