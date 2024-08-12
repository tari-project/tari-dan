export interface GetIdentityResponse {
    peer_id: string;
    public_key: string;
    public_addresses: Array<string>;
    supported_protocols: Array<string>;
    protocol_version: string;
    user_agent: string;
}
