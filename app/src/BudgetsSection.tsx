import { For, Show } from "solid-js";
import type { BudgetLimit, Budgets } from "./api";
import { formatLimit, parseTokens, withLimit, withoutWorktree } from "./budgetLogic";
import { norm } from "./paths";
import { setError, projects } from "./store";
import { B } from "./strings.budget";
import { wtName } from "./Usage";
import { Button, CheckboxField, Field, Input, Select } from "./ui";

type Patch = Partial<Record<keyof BudgetLimit, number | boolean | undefined>>;

/** The two limit fields and the auto-pause switch of one budget. A field that does not parse is refused and put back. */
function LimitRow(props: {
  title: string;
  hint?: string;
  limit: BudgetLimit | undefined;
  onChange: (p: Patch) => void;
  onRemove?: () => void;
  removeLabel?: string;
}) {
  const commit = (key: "daily" | "weekly", el: HTMLInputElement) => {
    const n = parseTokens(el.value);
    if (n === undefined) {
      setError(B.invalid(el.value));
      el.value = formatLimit(props.limit?.[key]);
      return;
    }
    el.value = formatLimit(n ?? undefined);
    if ((n ?? undefined) !== props.limit?.[key]) props.onChange({ [key]: n ?? undefined });
  };
  return (
    <div class="bud-row" role="group" aria-label={props.title}>
      <div class="bud-head">
        <strong>{props.title}</strong>
        <Show when={props.onRemove}>
          <Button aria-label={props.removeLabel} onClick={() => props.onRemove?.()}>
            {B.remove}
          </Button>
        </Show>
      </div>
      <Show when={props.hint}>
        <div class="auto-note">{props.hint}</div>
      </Show>
      <div class="set-fields">
        <Field label={B.daily} hint={B.limitHint}>
          <Input
            type="text"
            inputmode="text"
            value={formatLimit(props.limit?.daily)}
            spellcheck={false}
            onChange={(e) => commit("daily", e.currentTarget)}
          />
        </Field>
        <Field label={B.weekly} hint={B.limitHint}>
          <Input
            type="text"
            inputmode="text"
            value={formatLimit(props.limit?.weekly)}
            spellcheck={false}
            onChange={(e) => commit("weekly", e.currentTarget)}
          />
        </Field>
      </div>
      <CheckboxField checked={props.limit?.auto_pause === true} onChange={(v) => props.onChange({ auto_pause: v })}>
        {B.autoPause}
      </CheckboxField>
      <div class="auto-note bud-warn">{B.autoPauseWarn}</div>
    </div>
  );
}

/** Settings block: daily and weekly token limits for all worktrees and for single ones, each with an opt-in auto-pause. */
export function BudgetsSection(props: { budgets: Budgets | undefined; onSave: (b: Budgets) => void }) {
  const current = (): Budgets => props.budgets ?? {};
  const all = () =>
    projects().flatMap((p) => p.worktrees.map((w) => ({ path: w.path, label: `${p.name} / ${wtName(w.path)}` })));
  const own = () => Object.keys(current().worktrees ?? {});
  const labelOf = (path: string) => all().find((w) => norm(w.path) === norm(path))?.label ?? wtName(path);
  const free = () => all().filter((w) => !own().some((k) => norm(k) === norm(w.path)));
  return (
    <section class="set-sec">
      <h4>{B.title}</h4>
      <p class="auto-note">{B.note}</p>
      <Show when={props.budgets !== undefined} fallback={<div class="auto-note">{B.oldDaemon}</div>}>
        <LimitRow
          title={B.defaultRow}
          hint={B.defaultHint}
          limit={current().default}
          onChange={(p) => props.onSave(withLimit(current(), "default", p))}
        />
        <h5 class="bud-sub">{B.perWorktree}</h5>
        <p class="auto-note">{B.perWorktreeNote}</p>
        <For each={own()}>
          {(path) => (
            <LimitRow
              title={labelOf(path)}
              limit={current().worktrees?.[path]}
              onChange={(p) => props.onSave(withLimit(current(), { worktree: path }, p))}
              onRemove={() => props.onSave(withoutWorktree(current(), path))}
              removeLabel={B.removeLabel(labelOf(path))}
            />
          )}
        </For>
        <Show
          when={free().length > 0}
          fallback={
            <Show when={all().length === 0}>
              <div class="auto-note">{B.noWorktrees}</div>
            </Show>
          }
        >
          <Select
            value=""
            label={B.addWorktreeLabel}
            options={[{ value: "", label: B.addWorktree }, ...free().map((w) => ({ value: w.path, label: w.label }))]}
            onChange={(v) => v && props.onSave(withLimit(current(), { worktree: v }, {}))}
          />
        </Show>
      </Show>
    </section>
  );
}
