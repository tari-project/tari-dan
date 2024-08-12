import type { KeyBranch } from "./KeyBranch";
export interface KeysCreateRequest {
    branch: KeyBranch;
    specific_index: number | null;
}
