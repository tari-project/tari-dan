import type { ForeignProposalState } from "./ForeignProposalState";
import type { NodeHeight } from "./NodeHeight";
export interface ForeignProposal {
    shard_group: number;
    block_id: string;
    state: ForeignProposalState;
    proposed_height: NodeHeight | null;
    transactions: Array<string>;
    base_layer_block_height: number;
}
