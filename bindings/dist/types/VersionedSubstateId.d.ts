import type { SubstateId } from "./SubstateId";
export interface VersionedSubstateId {
    substate_id: SubstateId;
    version: number;
}
