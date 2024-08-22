import type { ComponentAddress } from "./ComponentAddress";
export interface AuthHook {
    component_address: ComponentAddress;
    method: string;
}
