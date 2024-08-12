import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
export interface AccountsTransferResponse {
    transaction_id: string;
    fee: Amount;
    fee_refunded: Amount;
    result: FinalizeResult;
}
