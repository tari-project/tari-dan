import type { Epoch } from "../Epoch";
export interface GetValidatorFeesRequest {
    validator_public_key: string;
    epoch: Epoch;
}
