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

import React from "react";
import { toHexString } from "../../VN/Components/helpers";
import {renderJson} from "../../../utils/helpers";

export default function Output({ shard, output }: { shard: string; output: any[] }) {

  return (
    <div id={shard} className="output">
      <b>Shard : </b>
      <span className="key">{shard}</span>
      <table>
        <thead>
          <tr>
            <th>Height</th>
            <th>Node hash</th>
              <th>Pledges</th>
              <th>Justify</th>
          </tr>
        </thead>
        <tbody>
          {output.map((row) => {
              let justify = JSON.parse(row.justify);
            return (
              <tr key={toHexString(row.node_hash)}>
                <td>{row.height}</td>
                <td className="key">{toHexString(row.node_hash)}</td>
                  <td>
                      <table>
                          <thead><tr>
                              <th>Shard</th>
                              <th>Current state</th>
                              <th>Pledged to</th>
                          </tr></thead>
                          <tbody>
                          { Array.isArray(justify.all_shard_pledges?.pledges) ? justify.all_shard_pledges.pledges.map((pledge:any) => {
                              // This enum gets serialized different ways... should be fixed in the rust
                              let currentState = Object.keys(pledge.pledge.current_state);
                                return (
                                    <tr key={pledge.shard_id}>
                                        <td>{pledge.shard_id}</td>
                                        <td>{currentState[0] !== "0" ? currentState[0] : pledge.pledge.current_state }</td>
                                        <td>{pledge.pledge.pledged_to_payload.id}</td>
                                    </tr>
                                )
                          }) : <tr><td>No pledges</td></tr> }

                          </tbody>
                      </table>
                  </td>
                  <td><pre style={{ height: "200px", overflow : "scroll"}}>{ row.justify ? renderJson(JSON.parse(row.justify)) :  ""}</pre></td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
