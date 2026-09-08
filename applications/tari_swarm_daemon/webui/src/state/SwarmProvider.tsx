//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import { ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { describeError, nodeRpc, swarmRpc } from "../api/rpc";
import {
  ConsensusStatus,
  EpochManagerStats,
  Identity,
  Indexer,
  Instance,
  InstanceType,
  LogFile,
  PoolRecord,
  StdoutFile,
  ValidatorNode,
  VnDetail,
  WalletDaemon,
} from "../types";
import { InstanceLogs, Swarm, SwarmContext, Toast } from "./context";

const LOGGED_TYPES: InstanceType[] = [
  "MinoTariNode",
  "MinoTariConsoleWallet",
  "MinoTariMiner",
  "TariValidatorNode",
  "TariIndexer",
  "TariWalletDaemon",
];

async function settle<T>(work: Promise<T>, fallback: T): Promise<T> {
  try {
    return await work;
  } catch {
    return fallback;
  }
}

/** Groups log files by the instance whose base path contains them. */
/**
 * Buckets log files by the instance id the daemon reported for each one.
 *
 * Instances do not have distinct directories: the wallet daemon and the key-creating run that seeds it share
 * `wallet-daemon-00`, so deducing the owner from the path handed both instances' files to whichever matched
 * first and left the other showing nothing.
 */
function assignToInstances(
  logFiles: LogFile[],
  stdoutFiles: StdoutFile[],
): Record<number, InstanceLogs> {
  const byInstance: Record<number, InstanceLogs> = {};

  for (const file of logFiles) {
    (byInstance[file[3]] ??= { logs: [], stdout: [] }).logs.push(file);
  }
  for (const file of stdoutFiles) {
    (byInstance[file[2]] ??= { logs: [], stdout: [] }).stdout.push(file);
  }
  return byInstance;
}

export function SwarmProvider({ children }: { children: ReactNode }) {
  const [instances, setInstances] = useState<Instance[]>([]);
  const [validators, setValidators] = useState<ValidatorNode[]>([]);
  const [wallets, setWallets] = useState<WalletDaemon[]>([]);
  const [indexers, setIndexers] = useState<Indexer[]>([]);
  const [details, setDetails] = useState<Record<number, VnDetail>>({});
  const [logs, setLogs] = useState<Record<number, InstanceLogs>>({});
  const [baseNodeHeight, setBaseNodeHeight] = useState<number | null>(null);
  const [isMining, setIsMining] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [intervalMs, setIntervalMsState] = useState(() =>
    Number(localStorage.getItem("swarm.interval") ?? 2000),
  );
  const [toasts, setToasts] = useState<Toast[]>([]);

  // Identities never change for the life of an instance, so they are fetched once.
  const identities = useRef<Record<number, Identity>>({});
  const nextToastId = useRef(0);
  const pollNow = useRef<() => void>(() => {});

  const report = useCallback((message: string) => {
    const id = (nextToastId.current += 1);
    setToasts((current) => [...current.slice(-3), { id, message }]);
    setTimeout(() => setToasts((current) => current.filter((t) => t.id !== id)), 8000);
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((t) => t.id !== id));
  }, []);

  const setIntervalMs = useCallback((ms: number) => {
    localStorage.setItem("swarm.interval", String(ms));
    setIntervalMsState(ms);
  }, []);

  const pollValidator = useCallback(async (vn: ValidatorNode): Promise<VnDetail> => {
    if (!vn.is_running) {
      return { epoch: null, consensus: null, pool: [], identity: null, error: null };
    }
    const url = vn.jrpc;
    try {
      if (!identities.current[vn.instance_id]) {
        identities.current[vn.instance_id] = (await nodeRpc(url, "get_identity")) as Identity;
      }
      const [epoch, consensus, pool] = await Promise.all([
        settle<EpochManagerStats | null>(nodeRpc(url, "get_epoch_manager_stats"), null),
        settle<ConsensusStatus | null>(nodeRpc(url, "get_consensus_status"), null),
        settle<{ tx_pool: PoolRecord[] }>(nodeRpc(url, "get_tx_pool"), { tx_pool: [] }),
      ]);
      return {
        epoch,
        consensus,
        pool: pool.tx_pool ?? [],
        identity: identities.current[vn.instance_id] ?? null,
        error: null,
      };
    } catch (err) {
      return {
        epoch: null,
        consensus: null,
        pool: [],
        identity: identities.current[vn.instance_id] ?? null,
        error: describeError(err),
      };
    }
  }, []);

  const poll = useCallback(async () => {
    const [allInstances, vnList, walletList, indexerList, mining] = await Promise.all([
      settle<{ instances: Instance[] }>(swarmRpc("list_instances", { by_type: null }), {
        instances: [],
      }),
      settle<{ nodes: ValidatorNode[] }>(swarmRpc("vns"), { nodes: [] }),
      settle<{ nodes: WalletDaemon[] }>(swarmRpc("list_wallet_daemons"), { nodes: [] }),
      settle<{ nodes: Indexer[] }>(swarmRpc("indexers"), { nodes: [] }),
      settle<{ result: boolean }>(swarmRpc("is_mining"), { result: false }),
    ]);

    setInstances(allInstances.instances);
    setValidators(vnList.nodes);
    setWallets(walletList.nodes);
    setIndexers(indexerList.nodes);
    setIsMining(mining.result);

    const baseNode = allInstances.instances.find(
      (i) => i.instance_type === "MinoTariNode" && i.is_running,
    );
    if (baseNode) {
      const resp = await settle<{ height: number | null }>(
        swarmRpc("get_minotari_node", { instance_id: baseNode.id }),
        { height: null },
      );
      setBaseNodeHeight(resp.height);
    } else {
      setBaseNodeHeight(null);
    }

    const detailList = await Promise.all(vnList.nodes.map(pollValidator));
    setDetails(
      Object.fromEntries(vnList.nodes.map((vn, i) => [vn.instance_id, detailList[i]])),
    );

    const logResults = await Promise.all(
      LOGGED_TYPES.map((instance_type) =>
        Promise.all([
          settle<LogFile[]>(swarmRpc("get_logs", { instance_type }), []),
          settle<StdoutFile[]>(swarmRpc("get_stdout", { instance_type }), []),
        ]),
      ),
    );
    setLogs(
      assignToInstances(
        logResults.flatMap(([l]) => l),
        logResults.flatMap(([, s]) => s),
      ),
    );
    setLoaded(true);
  }, [pollValidator]);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;

    // A recursive timeout rather than setInterval: a slow round never stacks on itself.
    const run = async () => {
      try {
        await poll();
      } catch (err) {
        if (!cancelled) {
          report(describeError(err));
        }
      }
      if (!cancelled && intervalMs > 0) {
        timer = window.setTimeout(run, intervalMs);
      }
    };

    pollNow.current = () => {
      window.clearTimeout(timer);
      void run();
    };
    void run();

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [poll, intervalMs, report]);

  const refresh = useCallback(() => pollNow.current(), []);

  const act = useCallback(
    async (label: string, run: () => Promise<unknown>) => {
      try {
        await run();
        pollNow.current();
      } catch (err) {
        report(`${label}: ${describeError(err)}`);
      }
    },
    [report],
  );

  const value = useMemo<Swarm>(
    () => ({
      instances,
      validators,
      wallets,
      indexers,
      details,
      logs,
      baseNodeHeight,
      isMining,
      loaded,
      intervalMs,
      setIntervalMs,
      refresh,
      act,
      toasts,
      dismiss,
      report,
    }),
    [
      instances,
      validators,
      wallets,
      indexers,
      details,
      logs,
      baseNodeHeight,
      isMining,
      loaded,
      intervalMs,
      setIntervalMs,
      refresh,
      act,
      toasts,
      dismiss,
      report,
    ],
  );

  return <SwarmContext.Provider value={value}>{children}</SwarmContext.Provider>;
}
