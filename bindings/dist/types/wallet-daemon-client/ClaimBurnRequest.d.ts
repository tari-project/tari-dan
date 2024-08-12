import type { Amount } from "../Amount";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface ClaimBurnRequest {
    account: ComponentAddressOrName | null;
    claim_proof: string;
    max_fee: Amount | null;
    key_id: number | null;
}
