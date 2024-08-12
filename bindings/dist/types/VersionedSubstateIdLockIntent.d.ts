import type { SubstateLockFlag } from "./SubstateLockFlag";
import type { VersionedSubstateId } from "./VersionedSubstateId";
export interface VersionedSubstateIdLockIntent {
    versioned_substate_id: VersionedSubstateId;
    lock_flag: SubstateLockFlag;
}
