export interface TransactionWaitResultRequest {
    transaction_id: string;
    timeout_secs: number | null;
}
