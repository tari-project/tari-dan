import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
export interface ClaimBurnResponse {
    transaction_id: string;
    fee: Amount;
    result: FinalizeResult;
}
