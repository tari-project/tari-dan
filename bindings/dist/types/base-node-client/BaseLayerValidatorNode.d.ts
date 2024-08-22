import type { SubstateAddress } from "../SubstateAddress";
export interface BaseLayerValidatorNode {
    public_key: string;
    shard_key: SubstateAddress;
    sidechain_id: string;
}
