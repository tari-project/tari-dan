// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { JrpcPermissions } from "./JrpcPermissions";

export interface Claims {
  id: number;
  name: string;
  permissions: JrpcPermissions;
  exp: bigint;
}
