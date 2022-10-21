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
import { toHexString } from "./helpers";
import { getRecentTransactions } from "./json_rpc";
import "./RecentTransactions.css";

interface IRecentTransaction {
  height: number;
  payload_height: number;
  payload_id: number[];
  shard: number[];
  total_leader_proposals: number;
  total_votes: number;
}

function RecentTransactions() {
  const [recentTransacations, setRecentTransacations] = useState<IRecentTransaction[]>([]);
  useEffect(() => {
    getRecentTransactions().then((response) => {
      setRecentTransacations(response.transactions);
    });
  }, []);
  if (recentTransacations === undefined) {
    return (
      <div className="section">
        <h4>Recent transactions ... loading</h4>
      </div>
    );
  }
  console.log(recentTransacations);
  return (
    <div className="section">
      <div className="caption">Recent transactions</div>
      <table className="recent-transactions-table">
        <tr>
          <th className="column">Height</th>
          <th className="column">Payload height</th>
          <th className="column">Payload id</th>
          <th className="column">Shard</th>
          <th className="column">Total leader proposal</th>
          <th className="column">Total votes</th>
        </tr>
        {recentTransacations.map(
          ({ height, payload_height, payload_id, shard, total_leader_proposals, total_votes }) => (
            <tr>
              <td>{height}</td>
              <td>{payload_height}</td>
              <td className="key">{toHexString(payload_id)}</td>
              <td className="key">{toHexString(shard)}</td>
              <td>{total_leader_proposals}</td>
              <td>{total_votes}</td>
            </tr>
          )
        )}
      </table>
    </div>
  );
}

export default RecentTransactions;
