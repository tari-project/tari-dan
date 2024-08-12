import type { SubstateId } from "../SubstateId";
export interface GetSubstateRequest {
    address: SubstateId;
    version: number;
}
