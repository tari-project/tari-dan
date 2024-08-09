import type { VNFunctionDef } from "./VNFunctionDef";
export interface TemplateAbi {
    template_name: string;
    functions: Array<VNFunctionDef>;
    version: string;
}
