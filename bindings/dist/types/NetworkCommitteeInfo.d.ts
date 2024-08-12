import type { CommitteeShardInfo } from "./CommitteeShardInfo";
import type { Epoch } from "./Epoch";
export interface NetworkCommitteeInfo<TAddr> {
    epoch: Epoch;
    committees: Array<CommitteeShardInfo<TAddr>>;
}
