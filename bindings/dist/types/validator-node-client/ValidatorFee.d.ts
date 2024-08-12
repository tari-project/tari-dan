import type { Epoch } from "../Epoch";
export interface ValidatorFee {
    validator_public_key: string;
    epoch: Epoch;
    block_id: string;
    total_fee_due: number;
    total_transaction_fee: number;
}
