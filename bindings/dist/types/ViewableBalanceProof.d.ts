export interface ViewableBalanceProof {
    elgamal_encrypted: Uint8Array;
    elgamal_public_nonce: Uint8Array;
    c_prime: Uint8Array;
    e_prime: Uint8Array;
    r_prime: Uint8Array;
    s_v: Uint8Array;
    s_m: Uint8Array;
    s_r: Uint8Array;
}
