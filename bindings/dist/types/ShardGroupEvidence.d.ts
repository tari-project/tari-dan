import type { SubstateAddress } from "./SubstateAddress";
import type { SubstateLockType } from "./SubstateLockType";
export interface ShardGroupEvidence {
    substates: Record<SubstateAddress, SubstateLockType>;
    prepare_qc: string | null;
    accept_qc: string | null;
}
