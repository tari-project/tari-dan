import type { ShardEvidence } from "./ShardEvidence";
import type { SubstateAddress } from "./SubstateAddress";
export interface Evidence {
    evidence: Record<SubstateAddress, ShardEvidence>;
}
