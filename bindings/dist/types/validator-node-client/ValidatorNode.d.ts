import type { Epoch } from "../Epoch";
import type { SubstateAddress } from "../SubstateAddress";
export interface ValidatorNode {
    address: string;
    public_key: string;
    shard_key: SubstateAddress;
    start_epoch: Epoch;
    end_epoch: Epoch;
    fee_claim_public_key: string;
}
