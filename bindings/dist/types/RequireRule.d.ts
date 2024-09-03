import type { RuleRequirement } from "./RuleRequirement";
export type RequireRule = {
    "Require": RuleRequirement;
} | {
    "AnyOf": Array<RuleRequirement>;
} | {
    "AllOf": Array<RuleRequirement>;
};
