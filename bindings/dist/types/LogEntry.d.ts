import type { LogLevel } from "./LogLevel";
export interface LogEntry {
    message: string;
    level: LogLevel;
}
