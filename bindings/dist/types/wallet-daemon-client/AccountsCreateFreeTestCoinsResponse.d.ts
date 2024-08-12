import type { Account } from "../Account";
import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
export interface AccountsCreateFreeTestCoinsResponse {
    account: Account;
    transaction_id: string;
    amount: Amount;
    fee: Amount;
    result: FinalizeResult;
    public_key: string;
}
