import type { FunctionDef } from "./FunctionDef";
export interface TemplateDefV1 {
    template_name: string;
    tari_version: string;
    functions: Array<FunctionDef>;
}
