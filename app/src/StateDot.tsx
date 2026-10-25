import { DOT_GLYPH, DOT_LABEL, DotState } from "./agentState";

/** Status dot: the colour stays, and each state also has its own shape plus a name for hover and screen readers. */
export function StateDot(props: { state: DotState; tip?: string | null }) {
  const label = () => props.tip || DOT_LABEL[props.state];
  return (
    <span
      class="dot glyph"
      classList={{
        on: props.state !== "none" && props.state !== "done",
        work: props.state === "working",
        wait: props.state === "waiting",
        await: props.state === "awaiting",
        stall: props.state === "stalled",
        attn: props.state === "attention",
        queued: props.state === "queued",
        done: props.state === "done",
      }}
      role={label() ? "img" : undefined}
      aria-label={label() || undefined}
      aria-hidden={label() ? undefined : "true"}
      title={label() || undefined}
    >
      {props.state === "none" ? null : (
        <svg width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d={DOT_GLYPH[props.state]} />
        </svg>
      )}
    </span>
  );
}
