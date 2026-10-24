import { ErrorBoundary, Show, type JSX } from "solid-js";
import { copyDiagnostics, logFrontendError } from "./api";
import { errorText } from "./appError";
import { formatError } from "./errorHooks";
import { Button } from "./ui";

/** What a section shows instead of itself when it threw while rendering: the rest of the app (terminals included) keeps running. */
export function Fallback(props: { name: string; error: unknown; reset: () => void; whole?: boolean }) {
  // the log gets the real error with its stack; the screen gets a sentence
  void logFrontendError(formatError("boundary", props.error, props.name)).catch((e) => console.debug("logFrontendError", e));
  return (
    <div class="placeholder" role="alert" style={props.whole ? { height: "100vh" } : undefined}>
      <div>
        {props.whole ? "O Axelstra encontrou um erro." : `Esta parte (${props.name}) encontrou um erro.`} Os terminais continuam a correr.
      </div>
      <div class="hints boundary-msg">
        {errorText(props.error)}
      </div>
      <div class="empty-actions">
        <Button variant="default" onClick={props.reset}>
          Tentar de novo
        </Button>
        <Button onClick={() => void copyDiagnostics().catch((e) => console.debug("copyDiagnostics", e))}>Copiar diagnóstico</Button>
        <Show when={props.whole}>
          <Button onClick={() => location.reload()}>Recarregar</Button>
        </Show>
      </div>
    </div>
  );
}

/** A boundary around one section of the screen (sidebar, right panel, terminal area) or, with `whole`, the app. */
export function Section(props: { name: string; whole?: boolean; children: JSX.Element }) {
  return (
    <ErrorBoundary fallback={(error, reset) => <Fallback name={props.name} error={error} reset={reset} whole={props.whole} />}>
      {props.children}
    </ErrorBoundary>
  );
}
