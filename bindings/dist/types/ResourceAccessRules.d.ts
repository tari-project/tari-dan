import type { AccessRule } from "./AccessRule";
export interface ResourceAccessRules {
    mintable: AccessRule;
    burnable: AccessRule;
    recallable: AccessRule;
    withdrawable: AccessRule;
    depositable: AccessRule;
    update_non_fungible_data: AccessRule;
}
