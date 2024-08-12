import type { VNArgDef } from "./VNArgDef";
export interface VNFunctionDef {
    name: string;
    arguments: Array<VNArgDef>;
    output: string;
    is_mut: boolean;
}
