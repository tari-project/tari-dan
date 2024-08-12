export interface VNAddPeerRequest {
    public_key: string;
    addresses: Array<string>;
    wait_for_dial: boolean;
}
