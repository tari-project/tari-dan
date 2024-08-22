import type { ExecuteResult } from "../ExecuteResult";
import type { SubstateRequirement } from "../SubstateRequirement";
export interface TransactionSubmitResponse {
    transaction_id: string;
    inputs: Array<SubstateRequirement>;
    result: ExecuteResult | null;
    json_result: Array<any> | null;
}
