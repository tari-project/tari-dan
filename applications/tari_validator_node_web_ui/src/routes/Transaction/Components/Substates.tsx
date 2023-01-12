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
import JsonTooltip from "../../../Components/JsonTooltip";
import { renderJson } from "../../../utils/helpers";
import {toHexString} from "../../VN/Components/helpers";

export default function Substates({ substates }: any) {
  if (substates.size == 0) {
    return <div className="caption">No substates</div>;
  }
  console.log(substates);
  substates.map((substate: any) => {
    // console.log("parsing json", substate.justify, JSON.parse(substate.justify));
  });
  return (
    <>
      <div className="caption">Substates</div>
      <table style={{ border: "1px solid gray"}}>
        <thead>
          <tr>
            <th>Shard</th>
            <th>Data</th>
            <th>Created</th>
            <th>Destroyed</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {substates.map((substate: any) => (
            <tr>
              <td>{toHexString(substate.shard_id)}</td>
              <td>
                <pre>
                    {renderJson(JSON.parse(substate.data))}
                </pre>
              </td>
              <td>
                <pre>
                    {substate.created_justify? renderJson(JSON.parse(substate.created_justify)) : ""}
                </pre>
              </td>
              <td>
                <pre>

                    { substate.destroyed_justify ? renderJson(JSON.parse(substate.destroyed_justify)) : ""}
                </pre>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
