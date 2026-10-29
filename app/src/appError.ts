import { L } from "./strings.logic";
import { msgText } from "./msg";
/** A failed daemon/host call: which method, and a message that is always readable text. `english` is the daemon's English
 *  text; when the daemon sent a message `code` (crates/core/src/msg.rs) and this app knows it, `message` (so every
 *  `e.message`, `String(e)` and `errorText`) reads in the UI language instead, following the language live. */
export class AppError extends Error {
  method: string;
  english: string;
  code?: string;
  params?: Record<string, unknown>;
  constructor(method: string, message: string, code?: string, params?: Record<string, unknown>) {
    super(message);
    this.name = "AppError";
    this.method = method;
    this.english = message;
    if (code) this.code = code;
    if (params) this.params = params;
    if (code)
      Object.defineProperty(this, "message", {
        get: () => msgText(this.code, this.params) ?? this.english,
        configurable: true,
      });
  }
  /** `String(e)` in the callers' catch blocks reads as the message, not as "AppError: ..." */
  override toString() {
    return this.message;
  }
}

/** The app dropped its own request (the "slow" notice's Cancel button): not a failure, so nothing shows it as one. */
export const CANCELLED = "rpc.cancelled";
export const isCancelled = (e: unknown): boolean => e instanceof AppError && e.code === CANCELLED;

/** Readable text for whatever was thrown: never "[object Object]". */
export const errorText = (e: unknown): string => {
  if (e instanceof Error) return e.message || e.name;
  if (typeof e === "string") return e;
  if (e && typeof e === "object") {
    const o = e as Record<string, unknown>;
    if (typeof o.code === "string") {
      const t = msgText(o.code, o.params as Record<string, unknown> | undefined);
      if (t) return t;
    }
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
export const toAppError = (method: string, e: unknown): AppError => {
  if (e instanceof AppError) return e;
  // the host rejects with `{message, code?, params?}` (app/src-tauri CallError)
  const o = e && typeof e === "object" && !(e instanceof Error) ? (e as Record<string, unknown>) : undefined;
  const msg = o && typeof o.message === "string" && o.message ? o.message : errorText(e);
  return new AppError(
    method,
    msg,
    typeof o?.code === "string" ? o.code : undefined,
    o?.params && typeof o.params === "object" ? (o.params as Record<string, unknown>) : undefined,
  );
};
