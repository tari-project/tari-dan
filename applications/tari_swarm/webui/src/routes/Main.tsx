//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import { ChangeEvent, useEffect, useState } from "react";
import { jsonRpc } from "../utils/json_rpc";
import { ExecutedTransaction } from "../Types.ts";

enum Executable {
  BaseNode = 1,
  Wallet = 2,
  Miner = 3,
  ValidatorNode = 4,
  Indexer = 5,
  DanWallet = 6,
  Templates = 7,
}

async function jsonRpc2(address: string, method: string, params: any = null) {
  let id = 0;
  id += 1;
  const response = await fetch(address, {
    method: "POST",
    body: JSON.stringify({
      method: method,
      jsonrpc: "2.0",
      id: id,
      params: params,
    }),
    headers: {
      "Content-Type": "application/json",
    },
  });
  const json = await response.json();
  if (json.error) {
    throw json.error;
  }
  return json.result;
}

function ExtraInfoVN({ name, url, setRow, addTxToPool, autoRefresh, state, horizontal }: {
  name: string,
  url: string,
  setRow: any,
  addTxToPool: any,
  autoRefresh: boolean,
  state: any,
  horizontal: boolean
}) {
  const [bucket, setBucket] = useState(null);
  const [epoch, setEpoch] = useState(null);
  const [height, setHeight] = useState(null);
  const [pool, setPool] = useState([]);
  const [copied, setCopied] = useState(false);
  const [missingTxStates, setMissingTxStates] = useState({}); // {tx_id: [vn1, vn2, ...]}
  const [publicKey, setPublicKey] = useState(null);
  const [peerId, setPeerId] = useState(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (autoRefresh) {
      const timer = setInterval(() => {
        setTick(tick + 1);
      }, 1000);
      return () => clearInterval(timer);
    }
  }, [tick, autoRefresh]);
  useEffect(() => {
    jsonRpc2(url, "get_epoch_manager_stats").then((resp) => {
      setRow(resp.committee_shard.shard + 1);
      setBucket(resp.committee_shard.shard);
      setHeight(resp.current_block_height);
      setEpoch(resp.current_epoch);
    }).catch((resp) => {
      console.error("err", resp);
    });
    jsonRpc2(url, "get_tx_pool").then((resp) => {
      setPool(resp.tx_pool);
      addTxToPool(resp.tx_pool.map((tx: any) => tx.transaction.id).sort());
    });
    jsonRpc2(url, "get_identity").then((resp) => {
      setPublicKey(resp.public_key);
      setPeerId(resp.peer_id);
    });
    let missing_tx = new Set();
    for (const k in state) {
      if (k != name && state[k].length > 0) {
        missing_tx = new Set([...missing_tx, ...state[k]]);
      }
    }
    const my_txs = new Set(state[name]);
    missing_tx = new Set([...missing_tx].filter((tx) => !my_txs.has(tx)));
    const promises = Array.from(missing_tx).map((tx) => jsonRpc2(url, "get_transaction", [tx])
      .then((resp) => resp.transaction as ExecutedTransaction)
      .catch((resp) => {
        throw { resp, tx };
      }));
    Promise.allSettled(promises).then((results) => {
      const newState: Map<string, any> = new Map();
      for (const result of results) {
        if (result.status == "fulfilled") {
          const tx = result.value;
          newState.set(tx.transaction.id, {
            known: true,
            abort_details: tx.abort_details,
            final_decision: tx.final_decision,
          });
        } else {
          newState.set(result.reason.tx, { known: false });
        }
      }
      if (JSON.stringify(newState) != JSON.stringify(missingTxStates)) {
        setMissingTxStates(newState);
      }
    });
    // for (let tx of missing_tx) {
    //   jsonRpc2(url, "get_transaction", [tx]).then((resp) => {
    //     setMissingTxStates((state) => ({ ...state, [tx]: { known: true, abort_details: resp.transaction.abort_details, final_decision: resp.transaction.final_decision } }));
    //     // console.log(resp);
    //   }).catch((resp) => { setMissingTxStates((state) => ({ ...state, [tx]: { know: false } })); });
    // }
  }, [tick, state]);
  const shorten = (str: string) => {
    if (str.length > 20) {
      return str.slice(0, 3) + "..." + str.slice(-3);
    }
    return str;
  };
  useEffect(() => {
    if (copied) {
      setTimeout(() => setCopied(false), 1000);
    }
  }, [copied]);
  const copyToClipboard = (str: string) => {
    setCopied(true);
    navigator.clipboard.writeText(str);
  };
  const showMissingTx = (missingTxStates: { [key: string]: any }) => {
    if (Object.keys(missingTxStates).length == 0) {
      return null;
    }
    return (
      <>
        <hr />
        <h3>Transaction from others TXs pools</h3>
        <div style={{
          display: "grid",
          gridAutoFlow: horizontal ? "column" : "row",
          gridTemplateRows: horizontal ? "auto auto auto auto" : "auto",
          gridTemplateColumns: horizontal ? "auto" : "auto auto auto auto",
        }}>
          <b>Tx Id</b>
          <b>Known</b>
          <b>Abort details</b>
          <b>Final decision</b>
          {Object.keys(missingTxStates).map((tx) => {
            const { known, abort_details, final_decision } = missingTxStates[tx];
            return (
              <>
                <div onClick={() => copyToClipboard(tx)}>{copied && "Copied" || shorten(tx)}</div>
                <div style={{ color: known ? "green" : "red" }}><b>{known && "Yes" || "No"}</b></div>
                <div>{abort_details || <i>unknown</i>}</div>
                <div>{final_decision || <i>unknown</i>}</div>
              </>
            );
          })}
        </div>
      </>);
  };
  const showPool = (pool: Array<any>) => {
    if (pool.length == 0) {
      return null;
    }
    return (<>
        <hr />
        <h3>Pool transaction</h3>
        <div style={{
          display: "grid",
          gridAutoFlow: horizontal ? "column" : "row",
          gridTemplateRows: horizontal ? "auto auto auto auto auto" : "auto",
          gridTemplateColumns: horizontal ? "auto" : "auto auto auto auto auto",
        }}>
          <b>Tx Id</b>
          <b>Ready</b>
          <b>Local_Decision</b>
          <b>Remote_Decision</b>
          <b>Stage</b>
          {pool.map((tx) => (
            <>
              <div
                onClick={() => copyToClipboard(tx.transaction.id)}>{copied && "Copied" || shorten(tx.transaction.id)}</div>
              <div>{tx.is_ready && "Yes" || "No"}</div>
              <div>{tx.local_decision || "_"}</div>
              <div>{tx.remote_decision || "_"}</div>
              <div>{tx.stage}</div>
            </>))}
        </div>
      </>
    );
  };
  return (
    <div style={{ whiteSpace: "nowrap" }}>
      <hr />
      <div style={{
        display: "grid",
        gridAutoFlow: "column",
        gridTemplateColumns: "auto auto",
        gridTemplateRows: "auto auto auto auto auto",
      }}>
        <div><b>Bucket</b></div>
        <div><b>Height</b></div>
        <div><b>Epoch</b></div>
        <div><b>Public key</b></div>
        <div><b>Peer id</b></div>
        <div>{bucket}</div>
        <div>{height}</div>
        <div>{epoch}</div>
        <div>{publicKey}</div>
        <div>{peerId}</div>
      </div>
      {showPool(pool)}
      {showMissingTx(missingTxStates)}
    </div>
  );
}

function ShowInfo(params: any) {
  const {
    children,
    executable,
    name,
    node,
    logs,
    stdoutLogs,
    showLogs,
    autoRefresh,
    updateState,
    state,
    horizontal,
  } = params;
  const [row, setRow] = useState(1);
  // const [unprocessedTx, setUnprocessedTx] = useState([]);
  const nameInfo = name && (
    <div>
      <pre></pre>
      <b>Name</b>
      {name}
    </div>
  );
  const jrpcInfo = node?.jrpc && (
    <div>
      <b>JRPC</b>
      <a href={`${node.jrpc}/json_rpc`} target="_blank">{node.jrpc}/json_rpc</a>
    </div>
  );
  const grpcInfo = node?.grpc && (
    <div>
      <b>GRPC</b>
      <span className="select">{node.grpc}</span>
    </div>
  );
  const httpInfo = node?.web && (
    <div>
      <b>HTTP</b>
      <a href={node.web} target="_blank">{node.web}</a>
    </div>
  );
  const logInfo = logs && (
    <>
      <div>
        <b>Logs</b>
        <div>
          {logs?.map((e: any) => (
            <div key={e[0]}>
              <a href={`log/${btoa(e[0])}/normal`}>
                {e[1]} - {e[2]}
              </a>
            </div>
          ))}
        </div>
      </div>
      <div>
        <div>
          {stdoutLogs?.map((e: any) => (
            <div key={e[0]}>
              <a href={`log/${btoa(e[0])}/stdout`}>stdout</a>
            </div>
          ))}
        </div>
      </div>
    </>
  );
  const addTxToPool = (tx: any) => {
    updateState({ name: name, state: tx });
  };
  return (
    <div className="info" key={name} style={{ gridRow: row }}>
      {nameInfo}
      {httpInfo}
      {jrpcInfo}
      {grpcInfo}
      {showLogs && logInfo}
      {executable === Executable.ValidatorNode && node?.jrpc &&
        <ExtraInfoVN name={name} url={node.jrpc} setRow={(new_row: any) => {
          if (new_row != row) setRow(new_row);
        }} addTxToPool={addTxToPool} autoRefresh={autoRefresh} state={state} horizontal={horizontal} />}
      {children}
    </div>
  );
}

function ShowInfos(params: any) {
  const { nodes, logs, stdoutLogs, name, showLogs, autoRefresh, horizontal } = params;
  const [state, setState] = useState<{ [key: string]: any }>({});
  let executable: Executable;
  switch (name) {
    case "vn":
      executable = Executable.ValidatorNode;
      break;
    case "dan":
      executable = Executable.DanWallet;
      break;
    case "indexer":
      executable = Executable.Indexer;
      break;
    default:
      console.log(`Unknown name ${name}`);
      break;
  }
  const updateState = (partial_state: { name: string, state: any }) => {
    if (JSON.stringify(state[partial_state.name]) != JSON.stringify(partial_state.state)) {
      setState((state) => ({ ...state, [partial_state.name]: partial_state.state }));
    }
  };
  return (
    <div className="infos" style={{ display: "grid" }}>
      {Object.keys(nodes).map((index) =>
        <ShowInfo key={index} executable={executable} name={nodes[index].name} node={nodes[index]}
                  logs={logs?.[`${name} ${index}`]} stdoutLogs={stdoutLogs?.[`${name} ${index}`]} showLogs={showLogs}
                  autoRefresh={autoRefresh} updateState={updateState} state={state} horizontal={horizontal} />)}
    </div>
  );
}

export default function Main() {
  const [vns, setVns] = useState({});
  const [danWallet, setDanWallets] = useState({});
  const [indexers, setIndexers] = useState({});
  const [node, setNode] = useState<{ grpc: any }>();
  const [wallet, _setWallet] = useState();
  const [logs, setLogs] = useState<any | null>({});
  const [stdoutLogs, setStdoutLogs] = useState<any | null>({});
  const [connectorSample, setConnectorSample] = useState(null);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [showLogs, setShowLogs] = useState(false);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [horizontal, setHorizontal] = useState(false);

  useEffect(() => {
    jsonRpc("vns")
      .then((resp) => {
        setVns(resp.nodes);
        Object.keys(resp.nodes).map((index) => {
          jsonRpc("get_logs", `vn ${index}`)
            .then((resp) => {
              setLogs((state: any) => ({ ...state, [`vn ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
          jsonRpc("get_stdout", `vn ${index}`)
            .then((resp) => {
              setStdoutLogs((state: any) => ({ ...state, [`vn ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
        });
      })
      .catch((error) => {
        console.log(error);
      });
    jsonRpc("dan_wallets")
      .then((resp) => {
        setDanWallets(resp.nodes);
        Object.keys(resp.nodes).map((index) => {
          jsonRpc("get_logs", `dan ${index}`)
            .then((resp) => {
              setLogs((state: any) => ({ ...state, [`dan ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
          jsonRpc("get_stdout", `dan ${index}`)
            .then((resp) => {
              setStdoutLogs((state: any) => ({ ...state, [`dan ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
        });
      })
      .catch((error) => {
        console.log(error);
      });
    jsonRpc("indexers")
      .then((resp) => {
        setIndexers(resp.nodes);
        Object.keys(resp.nodes).map((index) => {
          jsonRpc("get_logs", `indexer ${index}`)
            .then((resp) => {
              setLogs((state: any) => ({ ...state, [`indexer ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
          jsonRpc("get_stdout", `indexer ${index}`)
            .then((resp) => {
              setStdoutLogs((state: any) => ({ ...state, [`indexer ${index}`]: resp }));
            })
            .catch((error) => console.log(error));
        });
      })
      .catch((error) => {
        console.log(error);
      });
    jsonRpc("http_connector")
      .then((resp) => {
        setConnectorSample(resp);
      })
      .catch((error) => {
        console.log(error);
      });
    jsonRpc("get_logs", "node").then((resp) => {
      setLogs((state: any) => ({ ...state, node: resp }));
    });
    jsonRpc("get_logs", "wallet").then((resp) => {
      setLogs((state: any) => ({ ...state, wallet: resp }));
    });
    jsonRpc("get_logs", "miner").then((resp) => {
      setLogs((state: any) => ({ ...state, miner: resp }));
    });
    jsonRpc("get_stdout", "node").then((resp) => {
      setStdoutLogs((state: any) => ({ ...state, node: resp }));
    });
    jsonRpc("get_stdout", "wallet").then((resp) => {
      setStdoutLogs((state: any) => ({ ...state, wallet: resp }));
    });
    jsonRpc("get_stdout", "miner").then((resp) => {
      setStdoutLogs((state: any) => ({ ...state, miner: resp }));
    });
    jsonRpc("grpc_node").then((resp) => setNode({ grpc: resp }));
  }, []);

  const handleFileChange = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.item(0);
    if (file) {
      setSelectedFile(file);
    }
  };

  const handleFileUpload = () => {
    if (!selectedFile) {
      return;
    }
    const address = import.meta.env.VITE_DAEMON_JRPC_ADDRESS || "localhost:9000";
    const formData = new FormData();
    formData.append("file", selectedFile);
    fetch(`http://${address}/upload_template`, { method: "POST", body: formData }).then((resp) => {
      console.log("resp", resp);
    });
  };
  return (
    <div className="main">
      <button onClick={() => setShowLogs(!showLogs)}>{showLogs && "Hide" || "Show"} logs</button>
      <button onClick={() => setAutoRefresh(!autoRefresh)}>{autoRefresh && "Disable" || "Enable"} autorefresh</button>
      <button onClick={() => setHorizontal(!horizontal)}>Swap rows/columns</button>
      <div className="label">Base layer</div>
      <div className="infos">
        <ShowInfo executable={Executable.BaseNode} name="node" node={node} logs={logs?.node}
                  stdoutLogs={stdoutLogs?.node} showLogs={showLogs} horizontal={horizontal} />
        <ShowInfo executable={Executable.Wallet} name="wallet" node={wallet} logs={logs?.wallet}
                  stdoutLogs={stdoutLogs?.wallet} showLogs={showLogs} horizontal={horizontal} />
        <ShowInfo executable={Executable.Miner} name="miner" node={null} logs={logs?.miner}
                  stdoutLogs={stdoutLogs?.miner} showLogs={showLogs} horizontal={horizontal}>
          <button onClick={() => jsonRpc("mine", { num_blocks: 1 })}>Mine</button>
        </ShowInfo>
      </div>
      <div>
        <div className="label">Validator Nodes</div>
        <ShowInfos nodes={vns} logs={logs} stdoutLogs={stdoutLogs} name={"vn"} showLogs={showLogs}
                   autoRefresh={autoRefresh} horizontal={horizontal} />
      </div>
      <div>
        <div className="label">Dan Wallets</div>
        <ShowInfos nodes={danWallet} logs={logs} stdoutLogs={stdoutLogs} name={"dan"} showLogs={showLogs}
                   autoRefresh={autoRefresh} horizontal={horizontal} />
      </div>
      <div>
        <div className="label">Indexers</div>
        <ShowInfos nodes={indexers} logs={logs} stdoutLogs={stdoutLogs} name={"indexer"} showLogs={showLogs}
                   autoRefresh={autoRefresh} horizontal={horizontal} />
      </div>
      <div className="label">Templates</div>
      <div className="infos">
        <ShowInfo executable={Executable.Templates} horizontal={horizontal}>
          <input type="file" onChange={handleFileChange} />
          <button onClick={handleFileUpload}>Upload template</button>
        </ShowInfo>
      </div>
      {connectorSample && (
        <div className="label">
          <a href={connectorSample}>Connector sample</a>
        </div>
      )}
    </div>
  );
}
