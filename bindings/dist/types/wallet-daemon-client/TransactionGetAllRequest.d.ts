import type { ComponentAddress } from "../ComponentAddress";
import type { TransactionStatus } from "../TransactionStatus";
export interface TransactionGetAllRequest {
    status: TransactionStatus | null;
    component: ComponentAddress | null;
}
