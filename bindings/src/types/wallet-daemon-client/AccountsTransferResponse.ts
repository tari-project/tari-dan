// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";

export interface AccountsTransferResponse { transaction_id: string, fee: Amount, fee_refunded: Amount, result: FinalizeResult, }