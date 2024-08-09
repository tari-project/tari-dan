export interface AddPeerRequest {
    public_key: string;
    addresses: Array<string>;
    wait_for_dial: boolean;
}
