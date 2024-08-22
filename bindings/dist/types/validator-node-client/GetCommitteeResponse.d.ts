import type { Committee } from "../Committee";
import type { PeerAddress } from "../PeerAddress";
export interface GetCommitteeResponse {
    committee: Committee<PeerAddress>;
}
