import type { VNLogLevel } from "./VNLogLevel";
export interface VNLogEntry {
    timestamp: number;
    message: string;
    level: VNLogLevel;
}
