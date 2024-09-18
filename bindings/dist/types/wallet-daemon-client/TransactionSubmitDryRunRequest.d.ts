import type { SubstateRequirement } from "../SubstateRequirement";
import type { UnsignedTransaction } from "../UnsignedTransaction";
export interface TransactionSubmitDryRunRequest {
    transaction: UnsignedTransaction;
    signing_key_index: number | null;
    autofill_inputs: Array<SubstateRequirement>;
    detect_inputs: boolean;
    proof_ids: Array<number>;
}
