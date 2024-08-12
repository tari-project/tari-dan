import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface ListAccountNftRequest {
    account: ComponentAddressOrName | null;
    limit: number;
    offset: number;
}
