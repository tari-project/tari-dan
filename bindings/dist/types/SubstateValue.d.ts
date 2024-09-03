import type { ComponentHeader } from "./ComponentHeader";
import type { FeeClaim } from "./FeeClaim";
import type { NonFungibleContainer } from "./NonFungibleContainer";
import type { NonFungibleIndex } from "./NonFungibleIndex";
import type { Resource } from "./Resource";
import type { TransactionReceipt } from "./TransactionReceipt";
import type { UnclaimedConfidentialOutput } from "./UnclaimedConfidentialOutput";
import type { Vault } from "./Vault";
export type SubstateValue = {
    "Component": ComponentHeader;
} | {
    "Resource": Resource;
} | {
    "Vault": Vault;
} | {
    "NonFungible": NonFungibleContainer;
} | {
    "NonFungibleIndex": NonFungibleIndex;
} | {
    "UnclaimedConfidentialOutput": UnclaimedConfidentialOutput;
} | {
    "TransactionReceipt": TransactionReceipt;
} | {
    "FeeClaim": FeeClaim;
};
