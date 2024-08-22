import type { SubstateId } from "../SubstateId";
export interface InspectSubstateRequest {
    address: SubstateId;
    version: number | null;
}
