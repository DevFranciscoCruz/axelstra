import { L } from "./strings.logic";
/** A failed daemon/host call: which method, and a message that is always readable text. */
export class AppError extends Error {
  method: string;
  constructor(method: string, message: string) {
    super(message);
    this.name = "AppError";
    this.method = method;
  }
  /** `String(e)` in the callers' catch blocks reads as the message, not as "AppError: ..." */
  override toString() {
    return this.message;
  }
}

/** Readable text for whatever was thrown: never "[object Object]". */
export const errorText = (e: unknown): string => {
  if (e instanceof Error) return e.message || e.name;
  if (typeof e === "string") return e;
  if (e && typeof e === "object") {
    const o = e as Record<string, unknown>;
    for (const k of ["message", "error", "msg"]) if (typeof o[k] === "string" && o[k]) return o[k] as string;
    try {
      const s = JSON.stringify(e);
      if (s && s !== "{}") return s;
    } catch {
      /* circular: fall through */
    }
    return L.unknownError;
  }
  return e === undefined || e === null ? L.unknownError : String(e);
};

/** Wraps any failure of `method` as an AppError with a readable message. */
export const toAppError = (method: string, e: unknown): AppError =>
  e instanceof AppError ? e : new AppError(method, errorText(e));
