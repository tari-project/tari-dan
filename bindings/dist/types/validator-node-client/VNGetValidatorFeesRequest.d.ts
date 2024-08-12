import type { Epoch } from "../Epoch";
export interface VNGetValidatorFeesRequest {
    epoch_range: {
        start: Epoch;
        end: Epoch;
    };
    validator_public_key: string;
}
