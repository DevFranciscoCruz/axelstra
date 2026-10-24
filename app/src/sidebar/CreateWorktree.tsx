import { Modal } from "../Modal";
import { Select, Button, Field, Input } from "../ui";
import { hasIcon } from "../Icon";
import { agentLabel } from "../agentLabel";
import { agentKeys } from "../store";
import { S } from "../strings";
import { U } from "../strings.ui";

/** The "Novo worktree" dialog: the name (also the branch) and the agent started in it. */
export function CreateWorktree(props: {
  name: string;
  newName: string;
  setNewName: (v: string) => void;
  newAgent: string;
  setNewAgent: (v: string) => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  return (
    <Modal label={U.createWorktree.title} dirty={!!props.newName.trim()} onClose={props.onClose}>
      <div class="m-title font-semibold">{U.createWorktree.title}</div>
      <Field label={U.createWorktree.project} full>
        <Input value={props.name} disabled />
      </Field>
      <Field label={U.createWorktree.name} full>
        <Input
          placeholder={U.createWorktree.namePh}
          value={props.newName}
          onInput={(e) => props.setNewName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            // the dialog closes (and gives focus back to the "+" button) inside this handler: without this the Enter would then click that button and reopen the dialog empty
            e.preventDefault();
            if (props.newName.trim()) props.onSubmit();
          }}
          ref={(el) => queueMicrotask(() => el.focus())}
        />
      </Field>
      <Field label={U.createWorktree.agent} full>
        <Select
          value={props.newAgent}
          label={U.createWorktree.agent}
          options={[
            { value: "none", label: U.createWorktree.noAgent },
            ...agentKeys().map((a) => ({ value: a, label: agentLabel(a), icon: hasIcon(a) ? a : undefined })),
          ]}
          onChange={props.setNewAgent}
        />
      </Field>
      <div class="m-actions">
        <Button onClick={props.onClose}>{S.cancel}</Button>
        <Button variant="default" disabled={!props.newName.trim()} onClick={props.onSubmit}>
          {U.createWorktree.create}
        </Button>
      </div>
    </Modal>
  );
}
