import type { Epoch } from "./Epoch";
import type { Instruction } from "./Instruction";
import type { SubstateRequirement } from "./SubstateRequirement";
export interface UnsignedTransaction {
    fee_instructions: Array<Instruction>;
    instructions: Array<Instruction>;
    inputs: Array<SubstateRequirement>;
    min_epoch: Epoch | null;
    max_epoch: Epoch | null;
}
