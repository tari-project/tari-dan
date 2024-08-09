import type { IndexedValue } from "./IndexedValue";
import type { Type } from "./Type";
export interface InstructionResult {
    indexed: IndexedValue;
    return_type: Type;
}
