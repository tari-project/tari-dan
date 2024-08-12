import type { SubstateAddress } from "../SubstateAddress";
export interface TransactionClaimBurnResponse {
    transaction_id: string;
    inputs: Array<SubstateAddress>;
    outputs: Array<SubstateAddress>;
}
