export interface AuthLoginRequest {
    permissions: Array<string>;
    duration: {
        secs: number;
        nanos: number;
    } | null;
}
