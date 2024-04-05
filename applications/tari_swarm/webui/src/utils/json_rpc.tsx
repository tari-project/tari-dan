//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

// import { Mutex } from "async-mutex";

// let token: String | null = null;
let json_id = 0;
// const mutex_token = new Mutex();
// const mutex_id = new Mutex();

export async function jsonRpc(method: string, ...args: any[]) {
  const id = json_id;
  // await mutex_id.runExclusive(() => {
  //   id = json_id;
  json_id += 1;
  // })
  // let address = import.meta.env.VITE_DAEMON_JRPC_ADDRESS || "localhost:9000";
  // try {
  //   let text = await (await fetch("/json_rpc_address")).text();
  //   if (/^\d+(\.\d+){3}:[0-9]+$/.test(text)) {
  //     address = text;
  //   }
  // } catch {
  // }
  const address = import.meta.env.VITE_DAEMON_JRPC_ADDRESS || "localhost:9000";
  const headers: { [key: string]: string } = { "Content-Type": "application/json" };
  const response = await fetch(`http://${address}`, {
    method: "POST",
    body: JSON.stringify({
      method: method,
      jsonrpc: "2.0",
      id: id,
      params: [...args],
    }),
    headers: headers,
  });
  const json = await response.json();
  if (json.error) {
    console.error(method);
    console.error(...args);
    console.error(json.error);
    throw new Error(json.error);
  }
  return json.result;
}


