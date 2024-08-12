import type { Substate } from "../Substate";
import type { SubstateId } from "../SubstateId";
export interface IndexerGetSubstateResponse {
    address: SubstateId;
    version: number;
    substate: Substate;
    created_by_transaction: string;
}
