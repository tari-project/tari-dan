import type { RestrictedAccessRule } from "./RestrictedAccessRule";
export type AccessRule = "AllowAll" | "DenyAll" | {
    "Restricted": RestrictedAccessRule;
};
