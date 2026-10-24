import { Modal } from "../Modal";
import { Select, Button, Field, Input } from "../ui";
import { hasIcon } from "../Icon";
import { agentLabel } from "../agentLabel";
import { agentKeys } from "../store";

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
    <Modal label="Novo worktree" dirty={!!props.newName.trim()} onClose={props.onClose}>
      <div class="m-title font-semibold">Novo worktree</div>
      <Field label="Projeto" full>
        <Input value={props.name} disabled />
      </Field>
      <Field label="Nome" full>
        <Input
          placeholder="nome do worktree (também o nome do branch)"
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
      <Field label="Agente" full>
        <Select
          value={props.newAgent}
          label="Agente"
          options={[
            { value: "none", label: "Nenhum (só o worktree)" },
            ...agentKeys().map((a) => ({ value: a, label: agentLabel(a), icon: hasIcon(a) ? a : undefined })),
          ]}
          onChange={props.setNewAgent}
        />
      </Field>
      <div class="m-actions">
        <Button onClick={props.onClose}>Cancelar</Button>
        <Button variant="default" disabled={!props.newName.trim()} onClick={props.onSubmit}>
          Criar worktree
        </Button>
      </div>
    </Modal>
  );
}
