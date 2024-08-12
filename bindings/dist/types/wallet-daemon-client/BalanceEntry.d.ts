import type { Amount } from "../Amount";
import type { ResourceAddress } from "../ResourceAddress";
import type { ResourceType } from "../ResourceType";
import type { SubstateId } from "../SubstateId";
export interface BalanceEntry {
    vault_address: SubstateId;
    resource_address: ResourceAddress;
    balance: Amount;
    resource_type: ResourceType;
    confidential_balance: Amount;
    token_symbol: string | null;
}
