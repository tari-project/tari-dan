// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { VNConnectionDirection } from "./VNConnectionDirection";

export interface VNConnection { connection_id: string, peer_id: string, address: string, direction: VNConnectionDirection, age: {secs: number, nanos: number}, ping_latency: {secs: number, nanos: number} | null, user_agent: string | null, }