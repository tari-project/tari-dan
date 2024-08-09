import type { TransactionPoolRecord } from "../TransactionPoolRecord";
export interface GetTxPoolResponse {
    tx_pool: Array<TransactionPoolRecord>;
}
