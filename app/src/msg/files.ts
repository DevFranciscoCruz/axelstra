import type { Widen } from "../i18n";

/** Texts of the message codes raised by the daemon (see crates/core/src/msg.rs). `{name}` is filled from the call's params.
 *  pt-PT is the source; `en` and `ptBR` must have exactly the same keys (compile error otherwise, `msg.test.ts` repeats it). */
export const pt = {} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {};

/** Brazilian wording: written out in full, not a fallback to pt-PT. */
export const ptBR: Widen<typeof pt> = {};
