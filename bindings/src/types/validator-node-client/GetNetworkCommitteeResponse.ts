// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { CommitteeShardInfo } from "./CommitteeShardInfo";
import type { Epoch } from "../Epoch";

export interface GetNetworkCommitteeResponse {
  current_epoch: Epoch;
  committees: Array<CommitteeShardInfo>;
}
