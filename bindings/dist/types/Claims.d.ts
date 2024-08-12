import type { JrpcPermissions } from "./JrpcPermissions";
export interface Claims {
    id: number;
    name: string;
    permissions: JrpcPermissions;
    exp: number;
}
