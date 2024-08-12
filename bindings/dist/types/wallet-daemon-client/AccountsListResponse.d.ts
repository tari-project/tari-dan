import type { AccountInfo } from "./AccountInfo";
export interface AccountsListResponse {
    accounts: Array<AccountInfo>;
    total: number;
}
