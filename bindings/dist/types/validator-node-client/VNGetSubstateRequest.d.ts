import type { SubstateId } from "../SubstateId";
export interface VNGetSubstateRequest {
    address: SubstateId;
    version: number;
}
