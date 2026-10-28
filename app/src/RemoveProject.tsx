import { For, Show, createMemo, createSignal, createUniqueId, onMount, untrack } from "solid-js";
import { typedMatches } from "./Confirm";
import { Modal } from "./Modal";
import { type RemoveMode, removeProjectPreview } from "./api";
import { errorText } from "./appError";
import { type Preview, canChoose, effectLines, needsTyped, riskLines } from "./removeProjectLogic";
import { P } from "./strings.panels";
import { Button, CopyWord, Input, RadioGroup, cn } from "./ui";

type Pending = { id: string; name: string; resolve: (m: RemoveMode | null) => void };
const [pending, setPending] = createSignal<Pending | null>(null);

/** asks how to remove a project (only from the list, or everything); null when cancelled */
export const askRemoveProject = (id: string, name: string) =>
  new Promise<RemoveMode | null>((resolve) => {
    untrack(pending)?.resolve(null);
    setPending({ id, name, resolve });
  });

const answer = (m: RemoveMode | null) => {
  const p = pending();
  setPending(null);
  p?.resolve(m);
};

function Body(props: Pending) {
  const T = P.removeProject;
  const askId = createUniqueId();
  const [mode, setMode] = createSignal<RemoveMode>("list");
  const [prev, setPrev] = createSignal<Preview>(null);
  const [why, setWhy] = createSignal("");
  const [typed, setTyped] = createSignal("");
  // one read when the dialog opens: no polling
  onMount(() => {
    removeProjectPreview(props.id).then(setPrev, (e) => {
      setWhy(errorText(e));
      setPrev("error");
    });
  });
  const risks = createMemo(() => riskLines(prev()));
  const typedOk = () => !needsTyped(mode(), prev()) || typedMatches(props.name, typed());
  const ready = () => canChoose(mode(), prev()) && typedOk();
  return (
    <Modal label={T.title} role="alertdialog" describedBy={askId} onClose={() => answer(null)} closeOnBackdrop={false}>
      <div class="m-title font-semibold">{T.title}</div>
      <div id={askId} class="auto-note">
        {T.ask(props.name)}
      </div>
      <RadioGroup
        label={T.modes}
        value={mode()}
        options={[
          { value: "list", label: T.modeList },
          { value: "all", label: T.modeAll },
        ]}
        onChange={(v) => canChoose(v, prev()) && setMode(v)}
      />
      <Show when={prev() === null}>
        <div class="auto-note">{T.loading}</div>
      </Show>
      <Show when={prev() === "error"}>
        <div class="auto-note rm-risk">{T.previewFailed(why())}</div>
      </Show>
      <ul class="clean-list">
        <For each={effectLines(mode(), prev())}>{(l) => <li>{l}</li>}</For>
      </ul>
      <Show when={risks().length > 0}>
        <div class="rm-risk" role="alert">
          <div class="font-semibold">{T.riskTitle}</div>
          <ul class="clean-list">
            <For each={risks()}>{(l) => <li>{l}</li>}</For>
          </ul>
        </div>
      </Show>
      <Show when={needsTyped(mode(), prev())}>
        <div class="auto-note">{T.typeToConfirm(props.name)}</div>
        <CopyWord text={props.name} />
        <Input
          aria-label={T.typeToConfirm(props.name)}
          autocomplete="off"
          spellcheck={false}
          ref={(el: HTMLInputElement) => queueMicrotask(() => el.focus())}
          value={typed()}
          onInput={(e) => setTyped(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && ready() && answer(mode())}
        />
      </Show>
      <div class="m-actions justify-end">
        <Button onClick={() => answer(null)}>{T.cancel}</Button>
        <Button
          variant={mode() === "all" ? "destructive" : "default"}
          class={cn("primary", mode() === "all" && "kind-warn")}
          disabled={!ready()}
          onClick={() => answer(mode())}
        >
          {mode() === "all" ? T.okAll : T.okList}
        </Button>
      </div>
    </Modal>
  );
}

/** mounted once; renders whatever `askRemoveProject` queued */
export function RemoveProjectHost() {
  return (
    <Show when={pending()} keyed>
      {(p) => <Body {...p} />}
    </Show>
  );
}
