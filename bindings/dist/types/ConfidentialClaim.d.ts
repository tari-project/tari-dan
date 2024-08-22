import type { ConfidentialWithdrawProof } from "./ConfidentialWithdrawProof";
export interface ConfidentialClaim {
    public_key: string;
    output_address: string;
    range_proof: Array<number>;
    proof_of_knowledge: string;
    withdraw_proof: ConfidentialWithdrawProof | null;
}
