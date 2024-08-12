import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
export interface ConfidentialTransferResponse {
    transaction_id: string;
    fee: Amount;
    result: FinalizeResult;
}
