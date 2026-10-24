import { createSignal, untrack } from "solid-js";
import { hasIcon } from "./Icon";
import { agentLabel } from "./agentLabel";
import { Modal } from "./Modal";
import { agentKeys, projectAgent, setDefaultAgent } from "./store";
import { S } from "./strings";
import { Button, Field, Select } from "./ui";

/** "Novo agente": which agent to open. `done` gets the agent key, or null when dismissed; a choice becomes the default. */
export function AgentPickerModal(props: { project: string | undefined; done: (agent: string | null) => void }) {
  const [choice, setChoice] = createSignal(
    untrack(() => projectAgent(props.project)),
  ); /* the picker opens once per project */
  const go = () => {
    setDefaultAgent(choice());
    props.done(choice());
  };
  return (
    <Modal label={S.app.newAgent} onClose={() => props.done(null)}>
      <div class="m-heading">{S.app.newAgent}</div>
      <div class="m-full" onKeyDown={(e) => e.key === "Enter" && (e.ctrlKey || e.metaKey) && go()}>
        <Field label={S.app.agent} full>
          <Select
            value={choice()}
            label={S.app.agent}
            options={agentKeys().map((a) => ({
              value: a,
              label: agentLabel(a),
              desc: S.agentDescs[a],
              icon: hasIcon(a) ? a : undefined,
            }))}
            onChange={setChoice}
          />
        </Field>
      </div>
      <div class="m-actions">
        <Button onClick={() => props.done(null)}>{S.cancel}</Button>
        <Button variant="default" onClick={go} ref={(el) => queueMicrotask(() => el.focus())}>
          {S.app.open}
        </Button>
      </div>
    </Modal>
  );
}
