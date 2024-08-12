import type { Amount } from "./Amount";
export interface FeeClaim {
    epoch: number;
    validator_public_key: string;
    amount: Amount;
}
