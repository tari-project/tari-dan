import type { NonFungibleId } from "./NonFungibleId";
import type { VaultId } from "./VaultId";
export interface NonFungibleToken {
    vault_id: VaultId;
    nft_id: NonFungibleId;
    data: any;
    mutable_data: any;
    is_burned: boolean;
}
