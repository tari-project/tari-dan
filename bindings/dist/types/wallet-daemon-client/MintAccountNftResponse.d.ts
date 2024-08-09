import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
import type { NonFungibleId } from "../NonFungibleId";
import type { ResourceAddress } from "../ResourceAddress";
export interface MintAccountNftResponse {
    nft_id: NonFungibleId;
    resource_address: ResourceAddress;
    result: FinalizeResult;
    fee: Amount;
}
