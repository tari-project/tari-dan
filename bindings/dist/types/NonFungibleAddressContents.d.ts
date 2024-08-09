import type { NonFungibleId } from "./NonFungibleId";
import type { ResourceAddress } from "./ResourceAddress";
export interface NonFungibleAddressContents {
    resource_address: ResourceAddress;
    id: NonFungibleId;
}
