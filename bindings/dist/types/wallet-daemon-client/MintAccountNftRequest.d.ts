import type { Amount } from "../Amount";
import type { ComponentAddress } from "../ComponentAddress";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface MintAccountNftRequest {
    account: ComponentAddressOrName;
    metadata: string;
    mint_fee: Amount | null;
    create_account_nft_fee: Amount | null;
    existing_nft_component: ComponentAddress | null;
}
