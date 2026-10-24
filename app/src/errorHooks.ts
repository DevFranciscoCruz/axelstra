/** Webview-wide error capture: uncaught errors and unhandled rejections are forwarded to the Rust log. */

export type ErrorReport = { kind: string; message: string; stack: string };

const MAX_MESSAGE = 2000;
const MAX_STACK = 8000;

const clip = (s: string, n: number) => (s.length > n ? s.slice(0, n) : s);

/** Normalises whatever was thrown (Error, string, object, undefined) into a report. */
export function formatError(kind: string, value: unknown, where?: string): ErrorReport {
  let message: string;
  let stack = "";
  if (value instanceof Error) {
    message = `${value.name}: ${value.message}`;
    stack = value.stack ?? "";
  } else if (typeof value === "string") {
    message = value;
  } else {
    try {
      message = JSON.stringify(value) ?? String(value);
    } catch {
      message = String(value);
    }
  }
  if (where) stack = stack ? `${stack}\n    at ${where}` : `at ${where}`;
  return { kind, message: clip(message, MAX_MESSAGE), stack: clip(stack, MAX_STACK) };
}

/** Installs window.onerror + unhandledrejection; `send` must never throw into the page.
 *  `onRejection` also gets every unhandled rejection (the app shows it as an error toast instead of failing silently). */
export function installErrorHooks(send: (r: ErrorReport) => void, target: Window = window, onRejection?: (reason: unknown, r: ErrorReport) => void): void {
  const safe = (r: ErrorReport) => {
    try {
      send(r);
    } catch {
      /* logging must not cause errors */
    }
  };
  target.addEventListener("error", (e: ErrorEvent) => {
    const where = e.filename ? `${e.filename}:${e.lineno}:${e.colno}` : undefined;
    safe(formatError("error", e.error ?? e.message, where));
  });
  target.addEventListener("unhandledrejection", (e: PromiseRejectionEvent) => {
    const r = formatError("unhandledrejection", e.reason);
    safe(r);
    try {
      onRejection?.(e.reason, r);
    } catch {
      /* showing the failure must not cause errors */
    }
  });
}
