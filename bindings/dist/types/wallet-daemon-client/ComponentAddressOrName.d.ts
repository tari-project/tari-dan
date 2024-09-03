import type { ComponentAddress } from "../ComponentAddress";
export type ComponentAddressOrName = {
    "ComponentAddress": ComponentAddress;
} | {
    "Name": string;
};
