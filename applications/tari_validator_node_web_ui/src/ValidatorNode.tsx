import { useEffect, useState } from "react";
import AllVNs from "./AllVNs";
import Committees from "./Committees";
import Info from "./Info";
import { IEpoch, IIdentity } from "./interfaces";
import { getEpochManagerStats, getIdentity, getShardKey } from "./json_rpc";
import "./ValidatorNode.css";

function ValidatorNode() {
  const [epoch, setEpoch] = useState<IEpoch | undefined>(undefined);
  const [identity, setIdentity] = useState<IIdentity | undefined>(undefined);
  const [shardKey, setShardKey] = useState<string | null>(null);
  const [error, setError] = useState("");
  // Refresh every 2 minutes
  const refreshEpoch = (epoch: IEpoch | undefined) => {
    getEpochManagerStats()
      .then((response) => {
        if (response.current_epoch !== epoch?.current_epoch) {
          setEpoch(response);
        }
      })
      .catch((reason) => {
        console.log(reason);
        setError("Json RPC error, please check console");
      });
  };
  useEffect(() => {
    const id = window.setInterval(() => {
      refreshEpoch(epoch);
    }, 2 * 60 * 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [epoch]);
  // Initial fetch
  useEffect(() => {
    refreshEpoch(undefined);
    getIdentity()
      .then((response) => {
        setIdentity(response);
      })
      .catch((reason) => {
        console.log(reason);
        setError("Json RPC error, please check console");
      });
  }, []);
  // Get shard key.
  useEffect(() => {
    if (epoch !== undefined && identity !== undefined) {
      // The *10 is from the hardcoded constant in VN.
      getShardKey(epoch.current_epoch * 10, identity.public_key).then((response) => {
        setShardKey(response.shard_key);
      });
    }
  }, [epoch, identity]);
  if (error !== "") {
    return <div className="error">{error}</div>;
  }
  if (epoch === undefined || identity === undefined) return <div>Loading</div>;
  return (
    <div className="validator-node">
      <Info epoch={epoch} identity={identity} shardKey={shardKey} />
      {shardKey ? (
        <Committees currentEpoch={epoch.current_epoch} shardKey={shardKey} publicKey={identity.public_key} />
      ) : null}
      <AllVNs epoch={epoch.current_epoch} />
    </div>
  );
}

export default ValidatorNode;
