// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { ComponentAddressOrName } from "./ComponentAddressOrName";

export interface AccountsCreateFreeTestCoinsRequest {
  account: ComponentAddressOrName | null;
  amount: number;
  max_fee: number | null;
  key_id: number | null;
}