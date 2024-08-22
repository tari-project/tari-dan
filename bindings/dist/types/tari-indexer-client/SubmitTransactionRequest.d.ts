import type { SubstateRequirement } from "../SubstateRequirement";
import type { Transaction } from "../Transaction";
export interface SubmitTransactionRequest {
    transaction: Transaction;
    required_substates: Array<SubstateRequirement>;
    is_dry_run: boolean;
}
