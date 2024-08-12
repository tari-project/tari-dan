import type { LogLevel } from "./LogLevel";
export interface LogEntry {
    timestamp: number;
    message: string;
    level: LogLevel;
}
