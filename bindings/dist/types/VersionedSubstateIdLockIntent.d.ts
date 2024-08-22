import type { SubstateLockType } from "./SubstateLockType";
import type { VersionedSubstateId } from "./VersionedSubstateId";
export interface VersionedSubstateIdLockIntent {
    versioned_substate_id: VersionedSubstateId;
    lock_type: SubstateLockType;
}
