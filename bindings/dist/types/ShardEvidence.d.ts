import type { SubstateLockType } from "./SubstateLockType";
export interface ShardEvidence {
    prepare_justify: string | null;
    accept_justify: string | null;
    lock: SubstateLockType;
}
