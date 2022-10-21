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

import { useState } from "react";
import { IEpoch, IIdentity } from "./interfaces";
import { registerValidatorNode } from "./json_rpc";
import "./Info.css";

function Info({ epoch, identity, shardKey }: { epoch: IEpoch; identity: IIdentity; shardKey: string | null }) {
  const [registering, setRegistering] = useState(false);
  const [registerMessage, setRegisterMessage] = useState("");
  const register = () => {
    setRegistering(true);
    registerValidatorNode().then((response) => {
      if (response.message) {
        setRegistering(false);
        setRegisterMessage(response.message);
      } else {
        setRegisterMessage(`Registration successful, the TxId ${response.transaction_id}`);
      }
    });
  };
  const renderShardKey = () => {
    if (shardKey === null)
      return (
        <>
          <div className="label">Shard key</div>
          <div>
            <span
              className={`${registering ? "disabled-button" : "button"}`}
              id="register"
              onClick={registering ? () => {} : register}
            >
              Register
            </span>
            {registerMessage ? <span>{registerMessage}</span> : null}
          </div>
        </>
      );
    return (
      <tr>
        <td>Shard key</td>
        <td>{shardKey}</td>
      </tr>
    );
  };
  return (
    <div className="section">
      <div className="caption">Info</div>
      <table>
        <tr>
          <td>Epoch</td>
          <td>
            {epoch.current_epoch} ({epoch.is_valid ? "Valid" : "Not valid"})
          </td>
        </tr>
        <tr>
          <td>Node id</td>
          <td>{identity.node_id}</td>
        </tr>
        <tr>
          <td>Public address</td>
          <td>{identity.public_address}</td>
        </tr>
        <tr>
          <td>Public key</td>
          <td>{identity.public_key}</td>
        </tr>
        {renderShardKey()}
      </table>
    </div>
  );
}

export default Info;
