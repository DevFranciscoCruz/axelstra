import { Match, Switch } from "solid-js";
import { AddRemoteForm } from "./AddRemoteProject";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { pickProject } from "./projectActions";
import { addProjectTab, setAddProjectTab, setShowAddProject } from "./store";
import { S } from "./strings";
import { R } from "./strings.remote";
import { Button } from "./ui";

const option =
  "flex h-auto w-full flex-col items-start gap-0.5 whitespace-normal rounded-md border border-border p-3 text-left hover:bg-accent";

/** The one way in for a new project: a local folder (native picker) or a remote one (WSL / SSH form, as the second step with a Back button). */
export function AddProject() {
  const close = () => setShowAddProject(false);
  const local = () => {
    close();
    void pickProject();
  };
  return (
    <Modal label={R.addProjectTitle} class="modal" onClose={close}>
      <Switch>
        <Match when={addProjectTab() === "remote"}>
          <AddRemoteForm onClose={close} onBack={() => setAddProjectTab("choose")} />
        </Match>
        <Match when={true}>
          <div class="auto-head">
            <strong>{R.addProjectTitle}</strong>
            <span class="grow" />
            <Button onClick={close}>{S.close}</Button>
          </div>
          <div role="group" aria-label={R.chooseSource} class="flex flex-col gap-2">
            <Button type="button" variant="ghost" class={option} onClick={local}>
              <span class="flex items-center gap-2 text-base font-medium">
                <Icon name="folder" size={14} />
                {R.sourceLocal}
              </span>
              <span class="text-xs text-muted-foreground">{R.sourceLocalHint}</span>
            </Button>
            <Button type="button" variant="ghost" class={option} onClick={() => setAddProjectTab("remote")}>
              <span class="flex items-center gap-2 text-base font-medium">
                <Icon name="folder" size={14} />
                {R.sourceRemote}
              </span>
              <span class="text-xs text-muted-foreground">{R.sourceRemoteHint}</span>
            </Button>
          </div>
        </Match>
      </Switch>
    </Modal>
  );
}
