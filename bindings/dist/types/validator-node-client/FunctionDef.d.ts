import type { ArgDef } from "./ArgDef";
export interface FunctionDef {
    name: string;
    arguments: Array<ArgDef>;
    output: string;
    is_mut: boolean;
}
