import React, { useEffect, useState } from "react";
import { getAllVns } from "./json_rpc";

function AllVNs({ epoch }: { epoch: number }) {
  const [vns, setVns] = useState([]);
  useEffect(() => {
    getAllVns(epoch).then((response) => {
      setVns(response.vns);
    });
  }, [epoch]);
  if (vns.length === 0) return <div>All VNS are loading</div>;
  return (
    <>
      <div className="label">VNS</div>
      <div className="vns">
        {vns.map(({ public_key, shard_key }, i) => {
          return (
            <React.Fragment key={public_key}>
              <div className="label">Public key</div>
              <div className="key">{public_key}</div>
              <div className="label">Shard key</div>
              <div className="key">{shard_key}</div>
            </React.Fragment>
          );
        })}
      </div>
    </>
  );
}

export default AllVNs;
