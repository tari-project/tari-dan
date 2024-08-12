import type { SubstateId } from "./SubstateId";
export interface SubstateRequirement {
    substate_id: SubstateId;
    version: number | null;
}
