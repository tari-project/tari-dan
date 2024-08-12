import type { ConnectionDirection } from "./ConnectionDirection";
export interface Connection {
    connection_id: string;
    peer_id: string;
    address: string;
    direction: ConnectionDirection;
    age: {
        secs: number;
        nanos: number;
    };
    ping_latency: {
        secs: number;
        nanos: number;
    } | null;
    user_agent: string | null;
}
