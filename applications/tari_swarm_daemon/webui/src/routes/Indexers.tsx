//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import { NodeCard } from "../components/NodeCard";
import { useSwarm } from "../state/context";
import { Empty } from "../ui";

export default function Indexers() {
  const swarm = useSwarm();

  return (
    <div className="stack">
      <div className="page-head">
        <div>
          <h1>Indexers</h1>
          <p>Indexers follow the network and serve substate queries over REST.</p>
        </div>
      </div>

      {swarm.indexers.length ? (
        <div className="grid">
          {swarm.indexers.map((indexer) => (
            <NodeCard
              key={indexer.instance_id}
              id={indexer.instance_id}
              name={indexer.name}
              isRunning={indexer.is_running}
              links={
                <>
                  <a href={indexer.web} target="_blank" rel="noreferrer">
                    Web UI
                  </a>
                  <a href={indexer.api_url} target="_blank" rel="noreferrer">
                    REST
                  </a>
                </>
              }
            />
          ))}
        </div>
      ) : (
        <Empty>No indexers are configured.</Empty>
      )}
    </div>
  );
}
