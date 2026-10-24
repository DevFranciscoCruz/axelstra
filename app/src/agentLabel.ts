import { S } from "./strings";

/** Name shown for an agent key ("claude" -> "Claude Code"); an agent without a known name shows its key. */
export const agentLabel = (key: string): string => S.agentNames[key] ?? key;
