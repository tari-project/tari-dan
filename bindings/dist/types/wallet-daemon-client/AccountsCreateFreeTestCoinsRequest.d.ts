import type { Amount } from "../Amount";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface AccountsCreateFreeTestCoinsRequest {
    account: ComponentAddressOrName | null;
    amount: Amount;
    max_fee: Amount | null;
    key_id: number | null;
}
