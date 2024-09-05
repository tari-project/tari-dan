import type { RequireRule } from "./RequireRule";
export type RestrictedAccessRule = {
    Require: RequireRule;
} | {
    AnyOf: Array<RestrictedAccessRule>;
} | {
    AllOf: Array<RestrictedAccessRule>;
};
