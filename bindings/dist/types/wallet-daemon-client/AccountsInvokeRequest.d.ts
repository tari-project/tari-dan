import type { Amount } from "../Amount";
import type { Arg } from "../Arg";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface AccountsInvokeRequest {
    account: ComponentAddressOrName | null;
    method: string;
    args: Array<Arg>;
    max_fee: Amount | null;
}
