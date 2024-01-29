// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
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
  total_supply: number;
}
