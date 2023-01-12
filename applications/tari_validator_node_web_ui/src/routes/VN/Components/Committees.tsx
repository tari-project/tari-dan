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
import { getCommittee, getShardKey } from "../../../utils/json_rpc";
import Committee from "./Committee";
import { U256 } from "./helpers";
import PropTypes from "prop-types";

async function get_all_committees(currentEpoch: number, shardKey: string, publicKey: string) {
  let shardKeyMap: { [id: string]: string } = { [publicKey]: shardKey };
  let committee = await getCommittee(currentEpoch, shardKey);
  if (committee?.committee?.members === undefined) {
    return;
  }
  let nextShardSpace = new U256(shardKey).inc();
  let nextCommittee = await getCommittee(currentEpoch, nextShardSpace.n);
  let lastMemberShardKey;
  let shardSpaces: Array<[string, string, Array<string>]> = [];
  for (const member of committee.committee.members.concat(
    nextCommittee.committee.members[nextCommittee.committee.members.length - 1]
  )) {
    if (!(member in shardKeyMap)) {
      shardKeyMap[member] = (await getShardKey(currentEpoch * 10, member)).shard_key;
    }
    if (lastMemberShardKey !== undefined) {
      let end = new U256(shardKeyMap[member]).dec();
      shardSpaces.push([
        lastMemberShardKey,
        end.n,
        (await getCommittee(currentEpoch, lastMemberShardKey)).committee.members,
      ]);
    }
    lastMemberShardKey = shardKeyMap[member];
  }

  return shardSpaces;
}

function Committees({
  currentEpoch,
  shardKey,
  publicKey,
}: {
  currentEpoch: number;
  shardKey: string;
  publicKey: string;
}) {
  const [committees, setCommittees] = useState<Array<[string, string, Array<string>]>>([]);
  useEffect(() => {
    if (publicKey !== null) {
      get_all_committees(currentEpoch, shardKey, publicKey).then((response) => {
        if (response) setCommittees(response);
      });
    }
  }, [currentEpoch, shardKey, publicKey]);
  if (!committees) {
    return <div className="committees">Committees are loading</div>;
  }
  return (
    <div className="section">
      <div className="caption">Committees</div>
      <div className="committees">
        {committees.map(([begin, end, committee]) => (
          <Committee key={begin} begin={begin} end={end} members={committee} publicKey={publicKey} />
        ))}
      </div>
    </div>
  );
}

export default Committees;
