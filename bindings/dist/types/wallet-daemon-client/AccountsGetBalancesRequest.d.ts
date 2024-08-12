import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface AccountsGetBalancesRequest {
    account: ComponentAddressOrName | null;
    refresh: boolean;
}
