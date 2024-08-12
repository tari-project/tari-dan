import type { Epoch } from "../Epoch";
import type { VNCommitteeShardInfo } from "./VNCommitteeShardInfo";
export interface GetNetworkCommitteeResponse {
    current_epoch: Epoch;
    committees: Array<VNCommitteeShardInfo>;
}
