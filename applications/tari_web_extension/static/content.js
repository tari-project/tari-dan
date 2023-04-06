//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

var injected = false;

async function inject() {
      injected = true;
      let injectScript = await chrome.runtime.getURL("inject.js");
      const script = document.createElement("script");
      script.type = "text/javascript";
      script.src = injectScript;
      script.setAttribute("wasm", chrome.runtime.getURL("index_bg.wasm"));
      document.head.appendChild(script);
}

chrome.runtime.sendMessage({ event: 'isPageAllowed', url: (new URL(window.location.href)).hostname }).then((resp) => {
      if (resp) {
            inject();
      }
});

chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
      if (request?.event === "inject" && injected === false) {
            inject();
      }
})

