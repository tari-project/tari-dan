export interface ValidatorSignature {
    public_key: string;
    signature: {
        public_nonce: string;
        signature: string;
    };
}
