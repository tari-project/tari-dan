import type { IndexerConnectionDirection } from "./IndexerConnectionDirection";
export interface IndexerConnection {
    connection_id: string;
    peer_id: string;
    address: string;
    direction: IndexerConnectionDirection;
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
