import type { Substate } from "./Substate";
import type { SubstateId } from "./SubstateId";
export interface SubstateDiff {
    up_substates: Array<[SubstateId, Substate]>;
    down_substates: Array<[SubstateId, number]>;
}
