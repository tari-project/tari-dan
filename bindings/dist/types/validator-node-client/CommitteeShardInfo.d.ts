import type { SubstateAddress } from "../SubstateAddress";
import type { ValidatorNode } from "./ValidatorNode";
export interface CommitteeShardInfo {
    shard: number;
    substate_address_range: {
        start: SubstateAddress;
        end: SubstateAddress;
    };
    validators: Array<ValidatorNode>;
}
