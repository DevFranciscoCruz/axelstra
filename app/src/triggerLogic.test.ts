import { describe, expect, it, vi } from "vitest";
import type { TriggerFiring, TriggerLimits, TriggerPreview, TriggerState } from "./api";
import {
  type ToggleDeps,
  branchLabel,
  confirmMessage,
  cooldownPatch,
  eventText,
  firingKind,
  firingText,
  needsConfirm,
  perHourPatch,
  toggleTrigger,
} from "./triggerLogic";

const limits: TriggerLimits = {
  cooldown_secs: 1800,
  max_per_hour: 3,
  poll_secs: 60,
  cooldown_range: [60, 86400],
  max_per_hour_range: [1, 10],
};
const preview: TriggerPreview = {
  project: "p",
  prompt: "CI failed on the pull request of this worktree (branch ax/example)",
  template: "t",
  limits,
  agent: "claude",
  confirm_word: "ci_fix",
  never_main: true,
  never_pr_worktrees: true,
  audit_method: "trigger.ci_fix",
};
const state = (confirmed_at: number | null, ci_fix = false): TriggerState => ({
  project: "p",
  ci_fix,
  confirmed_at,
  limits,
  history: [],
  agent: "claude",
  confirm_word: "ci_fix",
  template: "t",
  example: "e",
});

describe("confirmation text", () => {
  it("lists the prompt, the limits, the exclusions and where it is logged", () => {
    const m = confirmMessage(preview);
    expect(m).toContain(preview.prompt);
    expect(m).toContain("a cada 60 s");
    expect(m).toContain("30 min");
    expect(m).toContain("3 por hora");
    expect(m).toContain("Nunca no worktree principal");
    expect(m).toContain("trigger.ci_fix");
    expect(m).toContain("nunca em modo sem pedidos de permissão");
  });
});

describe("toggleTrigger", () => {
  const deps = (over: { preview?: ToggleDeps["preview"]; ask?: ToggleDeps["ask"] } = {}) => ({
    preview: vi.fn<ToggleDeps["preview"]>(over.preview ?? (async () => preview)),
    ask: vi.fn<ToggleDeps["ask"]>(over.ask ?? (async () => true)),
    set: vi.fn<ToggleDeps["set"]>(async (_p, patch) => state(1, patch.ci_fix)),
  });

  it("the first enabling shows the preview in a type-to-confirm dialog and sends the word", async () => {
    const d = deps();
    const next = await toggleTrigger("p", true, state(null), d);
    expect(d.ask).toHaveBeenCalledTimes(1);
    const [msg, opts] = d.ask.mock.calls[0] as unknown as [string, { typeToConfirm: string; kind: string }];
    expect(msg).toContain(preview.prompt);
    expect(opts).toMatchObject({ typeToConfirm: "ci_fix", kind: "warning" });
    expect(d.set).toHaveBeenCalledWith("p", { ci_fix: true, confirm: "ci_fix" });
    expect(next?.ci_fix).toBe(true);
  });
  it("a refused dialog changes nothing", async () => {
    const d = deps({ ask: async () => false });
    expect(await toggleTrigger("p", true, state(null), d)).toBeNull();
    expect(d.set).not.toHaveBeenCalled();
  });
  it("an old daemon (no preview) changes nothing", async () => {
    const d = deps({ preview: async () => null });
    expect(await toggleTrigger("p", true, state(null), d)).toBeNull();
    expect(d.ask).not.toHaveBeenCalled();
  });
  it("later enablings and every disabling skip the dialog", async () => {
    const d = deps();
    await toggleTrigger("p", true, state(5), d);
    await toggleTrigger("p", false, state(null, true), d);
    expect(d.ask).not.toHaveBeenCalled();
    expect(d.set.mock.calls.map((c) => c[1])).toEqual([{ ci_fix: true }, { ci_fix: false }]);
  });
  it("needsConfirm is about confirmed_at only", () => {
    expect(needsConfirm({ confirmed_at: null })).toBe(true);
    expect(needsConfirm({ confirmed_at: 0 })).toBe(false);
  });
});

describe("events and history text", () => {
  it("names the branch without ax/", () => {
    expect(branchLabel("ax/fix-login")).toBe("fix-login");
    expect(eventText({ kind: "ci_fix", project: "p", worktree: "/w", branch: "ax/fix-login" })).toBe(
      "CI falhou em fix-login: a lançar tarefa de correção",
    );
    expect(
      eventText({ kind: "ci_fix_limited", project: "p", worktree: "/w", branch: "ax/a", detail: "rate limited" }),
    ).toContain("adiada (rate limited)");
    expect(
      eventText({ kind: "ci_fix_failed", project: "p", worktree: "/w", branch: "ax/a", detail: "agent missing" }),
    ).toContain("não consegui");
  });
  it("describes a firing", () => {
    const f: TriggerFiring = {
      ts: 1000,
      worktree: String.raw`C:\r\.axelstra-wt\app-fix`,
      sha: "abc",
      checks: ["build", "lint"],
      kind: "fired",
      detail: "",
    };
    expect(firingText(f, 1000 + 7200)).toBe("há 2 h · app-fix · build, lint");
    expect(firingKind(f)).toBe("lançada");
    expect(firingKind({ ...f, kind: "limited" })).toBe("adiada");
  });
});

describe("limit fields", () => {
  it("minutes become seconds within the daemon's range", () => {
    expect(cooldownPatch("10", limits)).toBe(600);
    expect(cooldownPatch("0,5", limits)).toBe(60);
    expect(cooldownPatch("0", limits)).toBe(60);
    expect(cooldownPatch("999999", limits)).toBe(86400);
    expect(cooldownPatch("", limits)).toBeNull();
    expect(cooldownPatch("abc", limits)).toBeNull();
  });
  it("the hourly maximum is clamped", () => {
    expect(perHourPatch("4", limits)).toBe(4);
    expect(perHourPatch("0", limits)).toBe(1);
    expect(perHourPatch("50", limits)).toBe(10);
    expect(perHourPatch(" ", limits)).toBeNull();
  });
});
