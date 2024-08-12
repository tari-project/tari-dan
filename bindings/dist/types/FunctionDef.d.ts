import type { ArgDef } from "./ArgDef";
import type { Type } from "./Type";
export interface FunctionDef {
    name: string;
    arguments: Array<ArgDef>;
    output: Type;
    is_mut: boolean;
}
