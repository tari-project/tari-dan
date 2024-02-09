// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { Amount } from "./Amount";
import type { FinalizeResult } from "./FinalizeResult";

export interface AccountsCreateFreeTestCoinsResponse {
  transaction_id: string;
  amount: Amount;
  fee: Amount;
  result: FinalizeResult;
  public_key: string;
}
