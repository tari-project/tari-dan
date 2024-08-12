import type { Amount } from "../Amount";
import type { ComponentAddressOrName } from "./ComponentAddressOrName";
export interface RevealFundsRequest {
    account: ComponentAddressOrName | null;
    amount_to_reveal: Amount;
    pay_fee_from_reveal: boolean;
    max_fee: Amount | null;
}
