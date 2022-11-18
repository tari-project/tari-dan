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

interface IRecentTransaction {
  height: number;
  payload_height: number;
  payload_id: number[];
  shard: number[];
  total_leader_proposals: number;
  total_votes: number;
}

interface ITableRecentTransaction {
  id: string;
  height: number;
  payload_height: number;
  payload_id: string;
  shard: string;
  total_leader_proposals: number;
  total_votes: number;
}

type ColumnKey = keyof ITableRecentTransaction;

function RecentTransactions() {
  const [recentTransacations, setRecentTransacations] = useState<ITableRecentTransaction[]>([]);
  const [lastSort, setLastSort] = useState({ column: "", order: -1 });
  useEffect(() => {
    getRecentTransactions().then((response) => {
      setRecentTransacations(
        response.transactions.map(
          ({ height, payload_height, payload_id, shard, total_leader_proposals, total_votes }: IRecentTransaction) => ({
            id: payload_height + toHexString(payload_id),
            height,
            payload_height,
            payload_id: toHexString(payload_id),
            shard: toHexString(shard),
            total_leader_proposals,
            total_votes,
          })
        )
      );
    });
  }, []);
  const sort = (column: ColumnKey) => {
    let order = 1;
    if (lastSort.column === column) {
      order = -lastSort.order;
    }
    setRecentTransacations(
      [...recentTransacations].sort((r0, r1) =>
        r0[column] > r1[column] ? order : r0[column] < r1[column] ? -order : 0
      )
    );
    setLastSort({ column, order });
  };
  if (recentTransacations === undefined) {
    return (
      <div className="section">
        <h4>Recent transactions ... loading</h4>
      </div>
    );
  }
  return (
    <div className="section">
      <div className="caption">Recent transactions</div>
      <table className="recent-transactions-table">
        <thead>
          <tr>
            <th className="column" onClick={() => sort("height")}>
              Height
              <span className="sort-indicator">
                {lastSort.column === "height" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("payload_height")}>
              Payload height
              <span className="sort-indicator">
                {lastSort.column === "payload_height" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("payload_id")}>
              Payload id
              <span className="sort-indicator">
                {lastSort.column === "payload_id" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("shard")}>
              Shard
              <span className="sort-indicator">
                {lastSort.column === "shard" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("total_leader_proposals")}>
              Total leader proposal
              <span className="sort-indicator">
                {lastSort.column === "total_leader_proposals" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("total_votes")}>
              Total votes
              <span className="sort-indicator">
                {lastSort.column === "total_votes" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
          </tr>
        </thead>
        <tbody>
          {recentTransacations.map(
            ({ id, height, payload_height, payload_id, shard, total_leader_proposals, total_votes }) => (
              <tr key={id}>
                <td>{height}</td>
                <td>{payload_height}</td>
                <td className="key">{payload_id}</td>
                <td className="key">{shard}</td>
                <td>{total_leader_proposals}</td>
                <td>{total_votes}</td>
              </tr>
            )
          )}
        </tbody>
      </table>
    </div>
  );
}

export default RecentTransactions;
