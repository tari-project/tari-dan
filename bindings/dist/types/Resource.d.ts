import type { Amount } from "./Amount";
import type { AuthHook } from "./AuthHook";
import type { Metadata } from "./Metadata";
import type { OwnerRule } from "./OwnerRule";
import type { ResourceAccessRules } from "./ResourceAccessRules";
import type { ResourceType } from "./ResourceType";
export interface Resource {
    resource_type: ResourceType;
    owner_rule: OwnerRule;
    owner_key: Array<number>;
    access_rules: ResourceAccessRules;
    metadata: Metadata;
    total_supply: Amount;
    view_key: string | null;
    auth_hook: AuthHook | null;
}