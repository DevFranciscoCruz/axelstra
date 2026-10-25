import { For, JSX, Show, createSignal, onCleanup } from "solid-js";
import {
  Badge,
  Button,
  Checkbox,
  CheckboxField,
  CheckboxInput,
  Dialog,
  Field,
  Input,
  RadioGroup,
  Select,
  Textarea,
} from "./index";

const BUTTON_VARIANTS = [
  "default",
  "outline",
  "secondary",
  "ghost",
  "destructive",
  "icon",
  "row",
  "chip",
  "link",
  "success",
] as const;
const BUTTON_SIZES = [
  "default",
  "sm",
  "icon",
  "tool",
  "glyph",
  "bare",
  "xs",
  "chip",
  "pill",
  "split",
  "splitEnd",
] as const;
const BADGE_VARIANTS = [
  "default",
  "agent",
  "high",
  "id",
  "warn",
  "bad",
  "quiet",
  "overlap",
  "pr",
  "prOk",
  "prErr",
  "prPend",
] as const;
const AGENTS = [
  { value: "claude", label: "Claude Code", icon: "claude", desc: "Anthropic" },
  { value: "codex", label: "Codex", icon: "codex", desc: "OpenAI" },
  { value: "gemini", label: "Gemini", icon: "gemini", desc: "Google" },
];

const OPTIONS = [
  { value: "a", label: "Primeira" },
  { value: "b", label: "Segunda" },
  { value: "c", label: "Terceira" },
];

const Section = (props: { title: string; children: JSX.Element }) => (
  <section class="mb-8">
    <h2 class="mb-3 border-b border-border pb-1 text-lg font-medium text-foreground">{props.title}</h2>
    <div class="flex flex-wrap items-start gap-3">{props.children}</div>
  </section>
);

const Cell = (props: { name: string; children: JSX.Element }) => (
  <div class="flex flex-col items-start gap-1">
    <div>{props.children}</div>
    <code class="text-xs text-muted-foreground">{props.name}</code>
  </div>
);

/** Reference page for everything in `ui/`, every variant side by side. Open the app with `#ui` in the URL. */
export function Gallery() {
  const root = document.documentElement;
  const before = root.dataset.theme;
  const [light, setLight] = createSignal(before === "light");
  const toggle = () => {
    setLight(!light());
    root.dataset.theme = light() ? "light" : "dark";
  };
  onCleanup(() => {
    if (before) root.dataset.theme = before;
  });
  const [radio, setRadio] = createSignal("a");
  const [pick, setPick] = createSignal("a");
  const [agent, setAgent] = createSignal("claude");
  const [field, setField] = createSignal(true);
  const [open, setOpen] = createSignal(false);

  return (
    <div class="h-screen overflow-auto bg-background p-6 text-base text-foreground">
      <header class="mb-6 flex items-center gap-3">
        <h1 class="text-xl font-medium">Componentes (ui/)</h1>
        <span class="flex-1" />
        <Button onClick={toggle} aria-pressed={light()}>
          {light() ? "Tema: claro" : "Tema: escuro"}
        </Button>
      </header>

      <Section title="Button: variant">
        <For each={BUTTON_VARIANTS}>
          {(v) => (
            <Cell name={v}>
              <Button variant={v}>Botão</Button>
            </Cell>
          )}
        </For>
      </Section>
      <Section title="Button: size">
        <For each={BUTTON_SIZES}>
          {(z) => (
            <Cell name={z}>
              <Button size={z}>Aa</Button>
            </Cell>
          )}
        </For>
      </Section>
      <Section title="Button: disabled">
        <Cell name="disabled">
          <Button disabled>Botão</Button>
        </Cell>
        <Cell name="default disabled">
          <Button variant="default" disabled>
            Botão
          </Button>
        </Cell>
      </Section>
      <Section title="Badge">
        <For each={BADGE_VARIANTS}>
          {(v) => (
            <Cell name={v}>
              <Badge variant={v}>{v}</Badge>
            </Cell>
          )}
        </For>
      </Section>
      <Section title="Input, Textarea, Field">
        <Cell name="Input">
          <Input placeholder="texto" />
        </Cell>
        <Cell name="Input disabled">
          <Input value="desativado" disabled />
        </Cell>
        <Cell name="Textarea">
          <Textarea rows={2} placeholder="várias linhas" />
        </Cell>
        <Cell name="Field">
          <Field label="Nome" hint="uma nota">
            <Input />
          </Field>
        </Cell>
      </Section>
      <Section title="Checkbox, CheckboxInput, CheckboxField">
        <Cell name="Checkbox">
          <Checkbox label="Opção" />
        </Cell>
        <Cell name="CheckboxInput">
          <CheckboxInput />
        </Cell>
        <Cell name="CheckboxInput disabled">
          <CheckboxInput disabled />
        </Cell>
        <Cell name="CheckboxField">
          <CheckboxField checked={field()} onChange={setField}>
            Opção
          </CheckboxField>
        </Cell>
      </Section>
      <Section title="RadioGroup">
        <Cell name="RadioGroup">
          <RadioGroup
            label="Escolha"
            value={radio()}
            options={OPTIONS.map((o) => ({ ...o, hint: "dica" }))}
            onChange={setRadio}
          />
        </Cell>
      </Section>
      <Section title="Select">
        <Cell name="Select">
          <Select value={pick()} options={OPTIONS} onChange={setPick} label="Escolha" class="w-40" />
        </Cell>
        <Cell name="Select (agents)">
          <Select value={agent()} options={AGENTS} onChange={setAgent} label="Agente" class="w-48" />
        </Cell>
      </Section>
      <Section title="Dialog">
        <Cell name="Dialog">
          <Button onClick={() => setOpen(true)}>Abrir</Button>
        </Cell>
        <Show when={open()}>
          <Dialog label="Exemplo" onClose={() => setOpen(false)}>
            <div class="auto-head">
              <strong>Exemplo</strong>
              <span class="grow" />
              <Button onClick={() => setOpen(false)}>Fechar</Button>
            </div>
          </Dialog>
        </Show>
      </Section>
    </div>
  );
}
