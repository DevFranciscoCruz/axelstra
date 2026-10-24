import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "./i18n";
import { ago, stampIso, stampLocal } from "./time";

afterEach(() => setLocale("pt-PT"));

describe("ago", () => {
  it("says now under a minute", () => {
    expect(ago(0)).toBe("agora");
    expect(ago(59)).toBe("agora");
    expect(ago(-5)).toBe("agora");
  });
  it("counts minutes, hours and days", () => {
    expect(ago(60)).toBe("há 1 min");
    expect(ago(59 * 60)).toBe("há 59 min");
    expect(ago(3600)).toBe("há 1 h");
    expect(ago(86399)).toBe("há 23 h");
    expect(ago(86400 * 3)).toBe("há 3 dias");
  });
  it("follows the UI language through Intl", () => {
    setLocale("en");
    expect(ago(0)).toBe("now");
    expect(ago(300)).toBe("5 min. ago");
    expect(ago(86400 * 3)).toBe("3 days ago");
    setLocale("pt-BR");
    expect(ago(300)).toBe("há 5 min.");
  });
});

describe("dates", () => {
  const at = new Date(2026, 9, 24, 13, 5).getTime() / 1000;
  it("are written in the UI language", () => {
    expect(stampLocal(at)).toBe("24/10/2026, 13:05");
    setLocale("en");
    expect(stampLocal(at)).toBe("Oct 24, 2026, 1:05 PM");
    setLocale("pt-BR");
    expect(stampLocal(at)).toBe("24 de out. de 2026, 13:05");
  });
  it("stay ISO for files that are exported, in every language", () => {
    for (const l of ["pt-PT", "en", "pt-BR"] as const) {
      setLocale(l);
      expect(stampIso(at)).toBe("2026-10-24 13:05");
    }
  });
});
