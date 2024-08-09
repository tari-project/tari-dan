import type { ElgamalVerifiableBalance } from "./ElgamalVerifiableBalance";
export interface ConfidentialOutput {
    commitment: string;
    stealth_public_nonce: string;
    encrypted_data: Array<number>;
    minimum_value_promise: number;
    viewable_balance: ElgamalVerifiableBalance | null;
}
