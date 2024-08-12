import type { Amount } from "../Amount";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
import type { Epoch } from "../Epoch";
export interface ClaimValidatorFeesRequest {
    account: ComponentAddressOrName | null;
    max_fee: Amount | null;
    validator_public_key: string;
    epoch: Epoch;
    dry_run: boolean;
}
