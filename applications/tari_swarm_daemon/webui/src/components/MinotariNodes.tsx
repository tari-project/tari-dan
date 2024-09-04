//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import React from "react";
import { jsonRpc } from "../utils/json_rpc.tsx";
import NodeControls from "./NodeControls.tsx";

interface Props {
  showLogs: boolean;
}

export default function MinotariNodes(props: Props) {
  const [nodes, setNodes] = React.useState<null | [any]>(null);
  const [isLoading, setIsLoading] = React.useState(true);


  React.useEffect(() => {
    jsonRpc("list_instances", { by_type: "MinoTariNode" }).then((nodes: any) => setNodes(nodes.instances))
      .then(() => setIsLoading(false));
  }, []);

  if (isLoading) {
    return <div>Loading...</div>;
  }

  return (
    <div>
      {nodes!.map((node: any, i: number) => (
        <Node key={i} {...node} showLogs={props.showLogs} />
      ))}
    </div>
  );
}

function Node(props: any) {
  const onStart = () => {
    jsonRpc("start_instance", { instance_id: props.id });
  };

  const onStop = () => {
    jsonRpc("stop_instance", { instance_id: props.id });
  };

  const onDeleteData = () => {
    jsonRpc("delete_instance_data", { instance_id: props.id });
  };

  return (
    <div className="info">
      <div>
        <b>Name</b>
        {props.name}
      </div>

      <div>
        <b>GRPC</b>
        {props.ports.grpc}
      </div>
      <NodeControls isRunning={props.is_running} onStart={onStart} onStop={onStop} onDeleteData={onDeleteData} />
      {props.showLogs && <div>TODO</div>}
    </div>
  );
}
