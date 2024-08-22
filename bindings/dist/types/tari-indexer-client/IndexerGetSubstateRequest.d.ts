import type { SubstateId } from "../SubstateId";
export interface IndexerGetSubstateRequest {
    address: SubstateId;
    version: number | null;
    local_search_only: boolean;
}
