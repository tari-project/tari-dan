import type { Amount } from "./Amount";
import type { ConfidentialStatement } from "./ConfidentialStatement";
export interface ConfidentialOutputStatement {
    output_statement: ConfidentialStatement | null;
    change_statement: ConfidentialStatement | null;
    range_proof: Array<number>;
    output_revealed_amount: Amount;
    change_revealed_amount: Amount;
}
