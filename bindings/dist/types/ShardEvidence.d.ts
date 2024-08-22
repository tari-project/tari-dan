import type { SubstateLockType } from "./SubstateLockType";
export interface ShardEvidence {
    qc_ids: Array<string>;
    lock: SubstateLockType;
}
