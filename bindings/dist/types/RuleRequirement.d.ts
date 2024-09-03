import type { ComponentAddress } from "./ComponentAddress";
import type { NonFungibleAddress } from "./NonFungibleAddress";
import type { ResourceAddress } from "./ResourceAddress";
export type RuleRequirement = {
    "Resource": ResourceAddress;
} | {
    "NonFungibleAddress": NonFungibleAddress;
} | {
    "ScopedToComponent": ComponentAddress;
} | {
    "ScopedToTemplate": Uint8Array;
};
