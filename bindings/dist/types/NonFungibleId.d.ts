export type NonFungibleId = {
    U256: Array<number>;
} | {
    String: string;
} | {
    Uint32: number;
} | {
    Uint64: number;
};
