//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

export type InstanceType =
  | "MinoTariNode"
  | "MinoTariConsoleWallet"
  | "MinoTariMiner"
  | "TariValidatorNode"
  | "TariIndexer"
  | "TariWalletDaemon"
  | "TariWalletDaemonCreateKey"
  | "TariSignalingServer";

export interface Instance {
  id: number;
  name: string;
  ports: Record<string, number>;
  settings: Record<string, string>;
  base_path: string;
  instance_type: InstanceType;
  is_running: boolean;
  is_config_dirty: boolean;
}

export interface ValidatorNode {
  instance_id: number;
  name: string;
  web: string;
  jrpc: string;
  is_running: boolean;
}

export interface WalletDaemon {
  instance_id: number;
  name: string;
  web: string;
  jrpc: string;
  is_running: boolean;
}

export interface Indexer {
  instance_id: number;
  name: string;
  web: string;
  api_url: string;
  graphql: string;
  is_running: boolean;
}

/** (full path, instance name, path without extension) */
/** [full path, instance name, path without extension, instance id] */
export type LogFile = [string, string, string, number];
/** (full path, "stdout") */
/** [full path, stream name, instance id] */
export type StdoutFile = [string, string, number];

export interface ShardGroup {
  start: number;
  end_inclusive: number;
}

export interface CommitteeInfo {
  shard_group: ShardGroup;
  num_shard_group_members: number;
}

export interface EpochManagerStats {
  current_epoch: number;
  current_block_height: number;
  current_block_hash: string;
  is_valid: boolean;
  is_initial_scanning_complete: boolean;
  start_epoch: number | null;
  committee_info: CommitteeInfo | null;
}

export interface ConsensusStatus {
  height: number;
  epoch: number;
  state: string;
}

export interface Identity {
  public_key: string;
  peer_id: string;
}

export interface PoolRecord {
  transaction_id: string;
  is_ready: boolean;
  stage: string;
  local_decision?: unknown;
  original_decision?: unknown;
  transaction?: { id: string };
}

/** What a validator holds for a transaction another validator has in its pool. */
export type TxHolding =
  | { kind: "pooled"; stage: string; ready: boolean }
  | { kind: "finalized"; decision: string }
  | { kind: "absent" };

/** Everything polled from one validator's own JSON-RPC. */
export interface VnDetail {
  epoch: EpochManagerStats | null;
  consensus: ConsensusStatus | null;
  pool: PoolRecord[];
  identity: Identity | null;
  error: string | null;
}
