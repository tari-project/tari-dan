import type { ComponentAddress } from "./ComponentAddress";
import type { ResourceAddress } from "./ResourceAddress";
import type { SubstateId } from "./SubstateId";
export type JrpcPermission = "AccountInfo" | {
    NftGetOwnershipProof: ResourceAddress | null;
} | {
    AccountBalance: SubstateId;
} | {
    AccountList: ComponentAddress | null;
} | "SubstatesRead" | "TemplatesRead" | "KeyList" | "TransactionGet" | {
    TransactionSend: SubstateId | null;
} | {
    GetNft: [SubstateId | null, ResourceAddress | null];
} | "StartWebrtc" | "Admin";
