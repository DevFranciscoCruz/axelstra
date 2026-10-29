import { describe, expect, it } from "vitest";
import { idleChoices, wakeFailedText } from "./hibernateLogic";
import { setLocale } from "./i18n";

describe("hibernation texts", () => {
  it("a failed wake reads the coded reason in the language of the app, else the English text", () => {
    setLocale("en");
    const d = {
      code: "hibernate.wake_failed",
      params: { id: 7, label: "Tarefa 1", detail: "exit 97" },
      error: "English fallback",
    };
    expect(wakeFailedText("Tarefa 1", d)).toContain("Tarefa 1 could not wake up");
    expect(wakeFailedText("Tarefa 1", d)).not.toContain("English fallback");
    expect(wakeFailedText("Tarefa 1", { code: "x.unknown", error: "plain reason" })).toContain("plain reason");
    setLocale("pt-PT");
  });
  it("the idle choices keep a saved odd value in order, once", () => {
    expect(idleChoices(30)).toEqual([5, 10, 15, 30, 60, 120, 240]);
    expect(idleChoices(45)).toEqual([5, 10, 15, 30, 45, 60, 120, 240]);
  });
});
