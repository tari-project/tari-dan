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

import React, { useEffect, useState } from "react";
import { getAllVns } from "../../../utils/json_rpc";

function AllVNs({ epoch }: { epoch: number }) {
  const [vns, setVns] = useState([]);
  useEffect(() => {
    getAllVns(epoch).then((response) => {
      setVns(response.vns);
    });
  }, [epoch]);
  if (!(vns?.length > 0)) return <div>All VNS are loading</div>;
  return (
    <div className="section">
      <div className="caption">VNs</div>
      <table className="all-vns-table">
        <thead>
          <tr>
            <th>Public key</th>
            <th>Shard key</th>
          </tr>
        </thead>
        <tbody>
          {vns.map(({ public_key, shard_key }, i) => (
            <tr key={public_key}>
              <td className="key">{public_key}</td>
              <td className="key">{shard_key}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default AllVNs;
