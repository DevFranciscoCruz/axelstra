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

/** Installs window.onerror + unhandledrejection; `send` must never throw into the page. */
export function installErrorHooks(send: (r: ErrorReport) => void, target: Window = window): void {
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
    safe(formatError("unhandledrejection", e.reason));
  });
}
