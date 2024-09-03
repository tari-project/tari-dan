//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import React from "react";
import { jsonRpc } from "../utils/json_rpc.tsx";
import NodeControls from "./NodeControls.tsx";

interface Props {
  showLogs: boolean;

}

export default function MinotariWallet(props: Props) {
  const [wallets, setWallets] = React.useState<null | [any]>(null);
  const [danWallets, setDanWallets] = React.useState<null | [any]>(null);
  const [isLoading, setIsLoading] = React.useState(true);


  React.useEffect(() => {
    jsonRpc("list_instances", { by_type: "MinoTariConsoleWallet" }).then((wallets: any) => setWallets(wallets.instances))
      .then(() => jsonRpc("list_instances", { by_type: "TariWalletDaemon" }).then((wallets: any) => setDanWallets(wallets.instances)))
      .then(() => setIsLoading(false));
  }, []);

  if (isLoading) {
    return <div>Loading...</div>;
  }

  return (
    <div>
      {wallets!.map((wallet: any, i: number) => (
        <Wallet key={i} {...wallet} showLogs={props.showLogs} danWallets={danWallets} />
      ))}
    </div>
  );
}

function Wallet(props: any) {
  const onStart = () => {
    jsonRpc("start_instance", { instance_id: props.id });
  };

  const onStop = () => {
    jsonRpc("stop_instance", { instance_id: props.id });
  };

  const onDeleteData = () => {
    jsonRpc("delete_instance_data", { instance_id: props.id });
  };

  const wallet = props.danWallets[0];

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
      {(wallet) ?
        <BurnFunds instanceId={props.id} danWallet={wallet} /> : <></>}
      {props.showLogs && <div>TODO</div>}
    </div>
  );
}

function BurnFunds(props: any) {
  const [amount, setAmount] = React.useState(1000);
  const [accountName, setAccountName] = React.useState<null | string>(null);
  const [claimUrl, setClaimUrl] = React.useState<null | string>(null);

  const onBurnFunds = () => {
    jsonRpc("burn_funds", {
      wallet_instance_id: props.danWallet.id,
      account_name: accountName,
      amount,
    }).then((res: any) => setClaimUrl(res.url));
  };

  return (
    <div>
      <pre>Burn to <b>{props.danWallet.name}</b>. This will mine 10 blocks.</pre>
      <input type="number" value={amount} placeholder="amount"
             onChange={(e) => setAmount(parseInt(e.target.value, 10))} />
      <input type="text" value={accountName || ""} placeholder="account name"
             onChange={(e) => setAccountName(e.target.value)} />
      <button onClick={onBurnFunds}>Burn funds</button>
      {claimUrl && <div>Claim data: <a href={claimUrl} target="_blank">{claimUrl}</a></div>}
    </div>
  );
}