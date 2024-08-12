import type { Committee } from "./Committee";
import type { SubstateAddress } from "./SubstateAddress";
export interface CommitteeShardInfo<TAddr> {
    shard: number;
    substate_address_range: {
        start: SubstateAddress;
        end: SubstateAddress;
    };
    validators: Committee<TAddr>;
}
