import { Show, createMemo, createSignal } from "solid-js";
import { AP } from "../strings.sources";
import { Button, Field, Input } from "../ui";
import { CloneProgress } from "./CloneProgress";
import { FolderField } from "./FolderField";
import { cloneActive, startClone } from "./cloneJob";
import { depthProblem, joinPath, lastParent, nameProblem, rememberParent, repoNameFromUrl, urlProblem } from "./logic";

/** Clone by address into `parent/name`; the name follows the address until the user edits it. */
export function CloneTab() {
  const [url, setUrl] = createSignal("");
  const [parent, setParent] = createSignal(lastParent());
  const [name, setName] = createSignal("");
  const [edited, setEdited] = createSignal(false);
  const [branch, setBranch] = createSignal("");
  const [depth, setDepth] = createSignal("");
  const [touched, setTouched] = createSignal(false);

  const folderName = () => (edited() ? name() : repoNameFromUrl(url()));
  const dest = () => (parent().trim() && folderName().trim() ? joinPath(parent(), folderName().trim()) : "");
  const problem = createMemo(() => {
    const u = urlProblem(url());
    if (u) return { field: "url", text: AP.urlProblem[u] };
    if (!parent().trim()) return { field: "parent", text: AP.folderRequired };
    const n = nameProblem(folderName());
    if (n) return { field: "name", text: AP.nameProblem[n] };
    if (depthProblem(depth())) return { field: "depth", text: AP.depthBad };
    return null;
  });
  const shown = (field: string) => (touched() && problem()?.field === field ? problem()!.text : "");
  const busy = () => cloneActive();

  const submit = () => {
    setTouched(true);
    if (problem() || busy()) return;
    rememberParent(parent());
    const d = depth().trim();
    void startClone({
      url: url().trim(),
      dest: dest(),
      ...(branch().trim() ? { branch: branch().trim() } : {}),
      ...(d ? { depth: Number(d) } : {}),
    });
  };

  return (
    <form
      class="flex flex-col gap-2"
      aria-label={AP.tab.clone}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <Field label={AP.url} hint={AP.urlHint} full>
        <Input
          value={url()}
          placeholder={AP.urlPh}
          autocomplete="off"
          spellcheck={false}
          disabled={busy()}
          aria-invalid={!!shown("url")}
          aria-describedby="ap-clone-url-msg"
          onInput={(e) => setUrl(e.currentTarget.value)}
        />
      </Field>
      <div id="ap-clone-url-msg" class="auto-note text-[var(--danger)]" role="alert">
        {shown("url")}
      </div>
      <FolderField
        id="ap-clone-parent"
        label={AP.folder}
        value={parent()}
        onChange={setParent}
        error={shown("parent")}
        disabled={busy()}
      />
      <Field label={AP.folderName} hint={AP.folderNameHint}>
        <Input
          value={folderName()}
          autocomplete="off"
          spellcheck={false}
          disabled={busy()}
          aria-invalid={!!shown("name")}
          aria-describedby="ap-clone-name-msg"
          onInput={(e) => {
            setEdited(true);
            setName(e.currentTarget.value);
          }}
        />
      </Field>
      <div id="ap-clone-name-msg" class="auto-note text-[var(--danger)]" role="alert">
        {shown("name")}
      </div>
      <div class="flex gap-2">
        <Field label={AP.branch}>
          <Input
            value={branch()}
            placeholder={AP.branchPh}
            autocomplete="off"
            spellcheck={false}
            disabled={busy()}
            onInput={(e) => setBranch(e.currentTarget.value)}
          />
        </Field>
        <Field label={AP.depth} hint={AP.depthHint}>
          <Input
            value={depth()}
            placeholder={AP.depthPh}
            inputmode="numeric"
            autocomplete="off"
            disabled={busy()}
            aria-invalid={!!shown("depth")}
            aria-describedby="ap-clone-depth-msg"
            onInput={(e) => setDepth(e.currentTarget.value)}
          />
        </Field>
      </div>
      <div id="ap-clone-depth-msg" class="auto-note text-[var(--danger)]" role="alert">
        {shown("depth")}
      </div>
      <Show when={dest()}>
        <div class="auto-note">{AP.destLine(dest())}</div>
      </Show>
      <CloneProgress />
      <div class="m-actions">
        <Button type="submit" variant="default" data-ap-primary="" disabled={busy()}>
          {busy() ? AP.cloning : AP.clone}
        </Button>
      </div>
    </form>
  );
}
