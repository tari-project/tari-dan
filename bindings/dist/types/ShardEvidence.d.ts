import type { SubstateLockFlag } from "./SubstateLockFlag";
export interface ShardEvidence {
    qc_ids: Array<string>;
    lock: SubstateLockFlag;
}
