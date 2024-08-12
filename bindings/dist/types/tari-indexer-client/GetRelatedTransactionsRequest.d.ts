import type { SubstateId } from "../SubstateId";
export interface GetRelatedTransactionsRequest {
    address: SubstateId;
    version: number | null;
}
