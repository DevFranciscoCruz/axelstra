import { ErrorBoundary, Show, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { copyDiagnostics, logFrontendError } from "./api";
import { errorText, isCancelled } from "./appError";
import { formatError } from "./errorHooks";
import { Button } from "./ui";
import { Icon } from "./Icon";
import { S } from "./strings";
import { U } from "./strings.ui";
import { P } from "./strings.panels";

/** What a section shows instead of itself when it threw while rendering: the rest of the app (terminals included) keeps running. */
export function Fallback(props: { name: string; error: unknown; reset: () => void; whole?: boolean }) {
  // the log gets the real error with its stack; the screen gets a sentence
  untrack(() => {
    void logFrontendError(formatError("boundary", props.error, props.name)).catch((e) =>
      console.debug("logFrontendError", e),
    );
  });
  const [copy, setCopy] = createSignal<"idle" | "ok" | "failed">("idle");
  let reset: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(reset));
  const copyDiag = () =>
    void copyDiagnostics().then(
      () => flash("ok"),
      (e) => (console.debug("copyDiagnostics", e), flash("failed")),
    );
  const flash = (s: "ok" | "failed") => {
    setCopy(s);
    clearTimeout(reset);
    reset = setTimeout(() => setCopy("idle"), 2000);
  };
  let root: HTMLDivElement | undefined;
  // the keyboard lands on the main action: Enter retries
  onMount(() => root?.querySelector<HTMLButtonElement>("[data-retry]")?.focus());
  return (
    <div ref={root} class="boundary" classList={{ whole: props.whole }} role="alert">
      <div class="boundary-card">
        <Icon name="warning" size={32} class="boundary-icon" />
        <h2 class="boundary-title">{props.whole ? U.boundary.whole : U.boundary.part(props.name)}</h2>
        <p class="boundary-note">{U.boundary.terminalsKeep}</p>
        <div class="boundary-msg">{errorText(props.error)}</div>
        <div class="empty-actions">
          <Button variant="default" data-retry onClick={props.reset}>
            {S.retry}
          </Button>
          <Button variant="secondary" onClick={copyDiag} aria-live="polite">
            {copy() === "ok" ? P.confirm.copied : copy() === "failed" ? U.boundary.copyDiagFailed : U.boundary.copyDiag}
          </Button>
          <Show when={props.whole}>
            <Button variant="secondary" onClick={() => location.reload()}>
              {U.boundary.reload}
            </Button>
          </Show>
        </div>
      </div>
    </div>
  );
}

/** A boundary around one section of the screen (sidebar, right panel, terminal area) or, with `whole`, the app. */
export function Section(props: { name: string; whole?: boolean; children: JSX.Element }) {
  // "Tentar de novo" bumps this so the children are built again (their resources fetch again), not just the error cleared
  const [attempt, setAttempt] = createSignal(0);
  return (
    <ErrorBoundary
      fallback={(error, reset) => {
        const again = () => (setAttempt((n) => n + 1), reset());
        // a request the user cancelled is not a failure: build the section again, show no error
        if (isCancelled(error)) return queueMicrotask(again), null;
        return <Fallback name={props.name} error={error} reset={again} whole={props.whole} />;
      }}
    >
      {(attempt(), props.children)}
    </ErrorBoundary>
  );
}
