import type { AccessRule } from "./AccessRule";
export type OwnerRule = "OwnedBySigner" | "None" | {
    ByAccessRule: AccessRule;
} | {
    ByPublicKey: Array<number>;
};
