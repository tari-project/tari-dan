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

import { useEffect, useState } from "react";
import { getConnections } from "../../../utils/json_rpc";
import { toHexString } from "./helpers";

interface IConnection {
  address: string;
  age: number;
  direction: boolean;
  node_id: number[];
  public_key: string;
}

function Connections() {
  const [connections, setConnections] = useState<IConnection[]>([]);
  useEffect(() => {
    getConnections().then((response) => {
      setConnections(response.connections);
    });
  }, []);

  return (
    <div className="section">
      <div className="caption">Connections</div>
      <table className="connections-table">
        <thead>
          <tr>
            <th>Address</th>
            <th>Age</th>
            <th>Direction</th>
            <th>Node id</th>
            <th>Public key</th>
          </tr>
        </thead>
        <tbody>
          {connections.map(({ address, age, direction, node_id, public_key }) => (
            <tr key={public_key}>
              <td>{address}</td>
              <td>{age}</td>
              <td>{direction ? "Inbound" : "Outbound"}</td>
              <td>{toHexString(node_id)}</td>
              <td>{public_key}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default Connections;
