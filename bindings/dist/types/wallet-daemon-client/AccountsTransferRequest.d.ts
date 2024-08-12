import type { Amount } from "../Amount";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
import type { ResourceAddress } from "../ResourceAddress";
export interface AccountsTransferRequest {
    account: ComponentAddressOrName | null;
    amount: Amount;
    resource_address: ResourceAddress;
    destination_public_key: string;
    max_fee: Amount | null;
    proof_from_badge_resource: string | null;
    dry_run: boolean;
}
