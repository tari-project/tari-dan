import type { SubstateLockType } from "./SubstateLockType";
import type { SubstateRequirement } from "./SubstateRequirement";
export interface SubstateRequirementLockIntent {
    substate_requirement: SubstateRequirement;
    version_to_lock: number;
    lock_type: SubstateLockType;
}
