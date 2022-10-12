import { useEffect, useState } from "react";
import Committee from "./Committee";
import { U256 } from "./helpers";
import { getCommittee, getShardKey } from "./json_rpc";

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
  if (committees.length === 0) {
    return <div className="commiittees">Committees are loading</div>;
  }
  return (
    <div className="committees">
      {committees.map(([begin, end, committee]) => (
        <Committee key={begin} begin={begin} end={end} members={committee} publicKey={publicKey} />
      ))}
    </div>
  );
}

export default Committees;
