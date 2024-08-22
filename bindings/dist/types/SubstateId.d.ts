import type { ComponentAddress } from "./ComponentAddress";
import type { FeeClaimAddress } from "./FeeClaimAddress";
import type { NonFungibleAddress } from "./NonFungibleAddress";
import type { NonFungibleIndexAddress } from "./NonFungibleIndexAddress";
import type { ResourceAddress } from "./ResourceAddress";
import type { TransactionReceiptAddress } from "./TransactionReceiptAddress";
import type { VaultId } from "./VaultId";
export type SubstateId = {
    Component: ComponentAddress;
} | {
    Resource: ResourceAddress;
} | {
    Vault: VaultId;
} | {
    UnclaimedConfidentialOutput: string;
} | {
    NonFungible: NonFungibleAddress;
} | {
    NonFungibleIndex: NonFungibleIndexAddress;
} | {
    TransactionReceipt: TransactionReceiptAddress;
} | {
    FeeClaim: FeeClaimAddress;
};
