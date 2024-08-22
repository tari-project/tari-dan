import type { ViewableBalanceProof } from "./ViewableBalanceProof";
export interface ConfidentialStatement {
    commitment: Array<number>;
    sender_public_nonce: Array<number>;
    encrypted_data: Array<number>;
    minimum_value_promise: number;
    viewable_balance_proof: ViewableBalanceProof | null;
}
