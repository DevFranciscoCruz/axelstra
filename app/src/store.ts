import { friendlyError } from "./friendly";
import { norm } from "./paths";
import { dismissToast, toast, toasts } from "./toasts";

// The shell state, split by topic; this file only re-exports it so `import ... from "./store"` keeps working.
export * from "./store/overlay";
export * from "./store/layout";
export * from "./store/notify";
export * from "./store/ui";
export * from "./store/sessions";
export * from "./store/tabs";
export * from "./store/git";
export * from "./store/hosts";
export * from "./store/memory";
export * from "./store/hibernate";

// `norm` lives in paths.ts (one for api and store); it stays importable from here
export { norm, friendlyError, toast, toasts, dismissToast };
export type { ToastInput } from "./toasts";
