import type { Epoch } from "./Epoch";
import type { NodeHeight } from "./NodeHeight";
import type { QuorumDecision } from "./QuorumDecision";
import type { ShardGroup } from "./ShardGroup";
import type { ValidatorSignature } from "./ValidatorSignature";
export interface QuorumCertificate {
    qc_id: string;
    block_id: string;
    block_height: NodeHeight;
    epoch: Epoch;
    shard_group: ShardGroup;
    signatures: Array<ValidatorSignature>;
    leaf_hashes: Array<string>;
    decision: QuorumDecision;
}
