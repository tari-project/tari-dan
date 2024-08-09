import type { ConfidentialOutputStatement } from "./ConfidentialOutputStatement";
export interface ConfidentialWithdrawProof {
    inputs: Array<Uint8Array>;
    input_revealed_amount: number;
    output_proof: ConfidentialOutputStatement;
    balance_proof: Array<number>;
}
