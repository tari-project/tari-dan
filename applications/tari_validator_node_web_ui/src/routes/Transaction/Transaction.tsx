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

import React, { useEffect } from "react";
import { Form, useLoaderData } from "react-router-dom";
import { getSubstates, getTransaction } from "../../utils/json_rpc";
import { fromHexString, toHexString } from "../VN/Components/helpers";
import Output from "./Components/Output";
import Substates from "./Components/Substates";
import Substate from "./Components/Substates";
import "./Transaction.css";

type loaderData = [string, Map<string, any[]>, Map<string, any[]>];

function splitToOutputs(transactions: any[]) {
  let shard_transactions = new Map<string, any[]>();
  for (let transaction of transactions) {
    let shard = toHexString(transaction.shard);
    if (!shard_transactions.has(shard)) {
      shard_transactions.set(shard, []);
    }
    shard_transactions.get(shard)?.push(transaction);
  }
  return shard_transactions;
}

export async function transactionLoader({ params }: { params: any }) {
  const outputs = splitToOutputs(await getTransaction(params.payloadId));
  let substates = new Map<string, any[]>();

  await Promise.all(
    Array.from(outputs.entries()).map(async ([shard, _]) => {
      substates.set(shard, await getSubstates(params.payloadId, shard));
    })
  );
  return [params.payloadId, substates, outputs];
}

export default function Transaction() {
  const [payloadId, substates, outputs] = useLoaderData() as loaderData;
  console.log(substates);
  return (
    <div className="transaction">
      <div className="payload-id caption">
        Payload ID
        <div className="key">{payloadId}</div>
      </div>
      <div>
        <b>Outputs : </b>
        {outputs?.size}
      </div>
      <div className="outputs">
        {Array.from(outputs.entries()).map(([shard, output]) => (
          <>
            <Output key={shard} shard={shard} output={output} />
            <Substates substates={substates.get(shard)} />
          </>
        ))}
      </div>
    </div>
  );
}
