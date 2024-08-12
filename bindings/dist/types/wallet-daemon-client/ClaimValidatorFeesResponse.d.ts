import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
export interface ClaimValidatorFeesResponse {
    transaction_id: string;
    fee: Amount;
    result: FinalizeResult;
}
