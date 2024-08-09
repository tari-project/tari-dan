import type { AccessRule } from "./AccessRule";
export interface ComponentAccessRules {
    method_access: Record<string, AccessRule>;
    default: AccessRule;
}
