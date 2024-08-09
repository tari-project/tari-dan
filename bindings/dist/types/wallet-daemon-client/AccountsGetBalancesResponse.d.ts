import type { BalanceEntry } from "./BalanceEntry";
import type { SubstateId } from "../SubstateId";
export interface AccountsGetBalancesResponse {
    address: SubstateId;
    balances: Array<BalanceEntry>;
}
