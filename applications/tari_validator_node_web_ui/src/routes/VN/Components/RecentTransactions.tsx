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
import { getRecentTransactions } from "../../../utils/json_rpc";
import { toHexString } from "./helpers";
import { Outlet, Link } from "react-router-dom";
import { renderJson } from "../../../utils/helpers";
import JsonTooltip from "../../../Components/JsonTooltip";

interface IRecentTransaction {
  payload_id: number[];
  timestamp: number;
  instructions: string;
  meta: string;
}

interface ITableRecentTransaction {
  id: string;
  payload_id: string;
  timestamp: Date;
  instructions: string;
  meta: string;
}

type ColumnKey = keyof ITableRecentTransaction;

function RecentTransactions() {
  const [recentTransacations, setRecentTransacations] = useState<ITableRecentTransaction[]>([]);
  const [lastSort, setLastSort] = useState({ column: "", order: -1 });
  useEffect(() => {
    getRecentTransactions().then((recentTransactions) => {
      setRecentTransacations(
        recentTransactions.map(({ instructions, meta, payload_id, timestamp }: IRecentTransaction) => ({
          id: toHexString(payload_id),
          payload_id: toHexString(payload_id),
          timestamp: new Date(timestamp * 1000),
          meta: meta,
          instructions: instructions,
        }))
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
            <th className="column" onClick={() => sort("payload_id")}>
              Payload id
              <span className="sort-indicator">
                {lastSort.column === "payload_id" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column" onClick={() => sort("timestamp")}>
              Timestamp
              <span className="sort-indicator">
                {lastSort.column === "shard" ? (lastSort.order === 1 ? "▲" : "▼") : ""}
              </span>
            </th>
            <th className="column">Meta</th>
            <th className="column">Instructions</th>
          </tr>
        </thead>
        <tbody>
          {recentTransacations.map(({ id, payload_id, timestamp, instructions, meta }) => (
            <tr key={id}>
              <td className="key">
                <Link to={`transaction/${payload_id}`}>{payload_id}</Link>
              </td>
              <td>{timestamp.toUTCString()}</td>
              <td>
                <JsonTooltip jsonText={meta}>Hover here</JsonTooltip>
              </td>
              <td>
                <JsonTooltip jsonText={instructions}>Hover here</JsonTooltip>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default RecentTransactions;
