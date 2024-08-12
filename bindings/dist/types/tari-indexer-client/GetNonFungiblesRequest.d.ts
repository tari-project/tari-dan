import type { SubstateId } from "../SubstateId";
export interface GetNonFungiblesRequest {
    address: SubstateId;
    start_index: number;
    end_index: number;
}
