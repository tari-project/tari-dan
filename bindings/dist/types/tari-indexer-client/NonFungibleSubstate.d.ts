import type { Substate } from "../Substate";
import type { SubstateId } from "../SubstateId";
export interface NonFungibleSubstate {
    index: number;
    address: SubstateId;
    substate: Substate;
}
