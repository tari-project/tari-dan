import type { SubstateId } from "../SubstateId";
export interface GetSubstateRequest {
    address: SubstateId;
    version: number | null;
    local_search_only: boolean;
}
