import { Show, createMemo, createSignal } from "solid-js";
import { projectInit } from "../api";
import { AppError, errorText } from "../appError";
import {
  navStamp,
  projects,
  refreshProjects,
  refreshSessions,
  selectIfStill,
  setShowAddProject,
  showNotice,
} from "../store";
import { AP } from "../strings.sources";
import { Button, CheckboxField, Field, Input } from "../ui";
import { FolderField } from "./FolderField";
import { joinPath, lastParent, nameProblem, rememberParent } from "./logic";

/** A new folder with `git init`, and optionally a README, a .gitignore and the first commit. */
export function NewTab() {
  const [parent, setParent] = createSignal(lastParent());
  const [name, setName] = createSignal("");
  const [readme, setReadme] = createSignal(true);
  const [gitignore, setGitignore] = createSignal(false);
  const [commit, setCommit] = createSignal(true);
  const [touched, setTouched] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal("");

  const problem = createMemo(() => {
    if (!parent().trim()) return { field: "parent", text: AP.folderRequired };
    const n = nameProblem(name());
    return n ? { field: "name", text: AP.nameProblem[n] } : null;
  });
  const shown = (field: string) => (touched() && problem()?.field === field ? problem()!.text : "");
  // a commit needs something to commit
  const canCommit = () => readme() || gitignore();

  const submit = async () => {
    setTouched(true);
    setFailure("");
    if (problem() || busy()) return;
    setBusy(true);
    try {
      rememberParent(parent());
      const stamp = navStamp();
      const created = await projectInit({
        parent: parent().trim(),
        name: name().trim(),
        readme: readme(),
        gitignore: gitignore(),
        commit: commit() && canCommit(),
      });
      await refreshProjects();
      const main = projects()
        .find((p) => p.id === created.id)
        ?.worktrees.find((w) => w.is_main);
      if (main) selectIfStill(stamp, main.path);
      void refreshSessions();
      showNotice(AP.created(created.name));
      setShowAddProject(false);
    } catch (e) {
      // the folder and `git init` are done even without a git identity: show it as it is
      if (e instanceof AppError && e.code === "project.init_no_identity") {
        await refreshProjects().catch(() => undefined);
        setFailure(AP.noIdentity);
      } else setFailure(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      class="flex flex-col gap-2"
      aria-label={AP.tab.new}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <FolderField
        id="ap-new-parent"
        label={AP.folder}
        value={parent()}
        onChange={setParent}
        error={shown("parent")}
        disabled={busy()}
      />
      <Field label={AP.newName}>
        <Input
          value={name()}
          placeholder={AP.newNamePh}
          autocomplete="off"
          spellcheck={false}
          disabled={busy()}
          aria-invalid={!!shown("name")}
          aria-describedby="ap-new-name-msg"
          onInput={(e) => setName(e.currentTarget.value)}
        />
      </Field>
      <div id="ap-new-name-msg" class="auto-note text-[var(--err)]" role="alert">
        {shown("name")}
      </div>
      <Show when={parent().trim() && name().trim() && !problem()}>
        <div class="auto-note">{AP.destLine(joinPath(parent(), name().trim()))}</div>
      </Show>
      <div role="group" aria-label={AP.tab.new} class="flex flex-col gap-1.5">
        <CheckboxField checked={readme()} onChange={setReadme}>
          {AP.readme}
        </CheckboxField>
        <CheckboxField checked={gitignore()} onChange={setGitignore}>
          {AP.gitignore}
        </CheckboxField>
        <Show when={canCommit()}>
          <CheckboxField checked={commit()} onChange={setCommit}>
            {AP.firstCommit}
          </CheckboxField>
        </Show>
      </div>
      <Show when={failure()}>
        <div role="alert" class="text-[var(--err)]">
          {failure()}
        </div>
      </Show>
      <div class="m-actions">
        <Button type="submit" variant="default" data-ap-primary="" disabled={busy()}>
          {busy() ? AP.creating : AP.create}
        </Button>
      </div>
    </form>
  );
}
