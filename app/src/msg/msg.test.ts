import { describe, expect, it } from "vitest";
import { coded, fill, msgText, MSG, parts } from "./index";
import { AppError, errorText, toAppError } from "../appError";
import { LOCALES, setLocale, leafPaths } from "../i18n";

// @ts-expect-error the app has no node typings; the test runs under node, from app/
import { readdirSync, readFileSync, statSync } from "node:fs";

/** every .rs file under `dir` (vite refuses to glob outside the app, so the Rust sources are read from disk) */
const rustFiles = (dir: string): string[] =>
  (readdirSync(dir) as string[]).flatMap((n) => {
    const p = `${dir}/${n}`;
    return statSync(p).isDirectory() ? rustFiles(p) : n.endsWith(".rs") ? [p] : [];
  });
const rust: string[] = ["../crates/core/src", "../crates/daemon/src", "../crates/cli/src", "src-tauri/src"].flatMap(
  (d) => rustFiles(d).map((f) => readFileSync(f, "utf8") as string),
);

/** every message code the Rust sources raise: `coded!("x.y", ...)`, `Coded::new("x.y", ...)`, `notice_coded("x.y"`... */
const rustCodes = (): string[] => {
  const out = new Set<string>();
  for (const src of rust)
    for (const m of src.matchAll(/(?:coded!\(|Coded::new\(|notice_coded\()\s*"([a-z0-9_.]+)"/g))
      if (!m[1]!.startsWith("test.")) out.add(m[1]!);
  return [...out].sort();
};

const names = (t: string) => [...t.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
const pt = Object.assign({}, ...Object.values(parts).map((p) => p.pt)) as Record<string, string>;
const en = Object.assign({}, ...Object.values(parts).map((p) => p.en)) as Record<string, string>;
const ptBR = Object.assign({}, ...Object.values(parts).map((p) => p.ptBR)) as Record<string, string>;

describe("message codes", () => {
  it("every code the daemon raises has a pt-PT, English and pt-BR text", () => {
    const missing = rustCodes().flatMap((c) =>
      (["pt", "en", "ptBR"] as const).filter((l) => !{ pt, en, ptBR }[l][c]).map((l) => `${c} (${l})`),
    );
    expect(missing).toEqual([]);
  });

  it("the three languages have the same keys and the same placeholders, and no key sits in two files", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(pt).sort());
    expect(Object.keys(ptBR).sort()).toEqual(Object.keys(pt).sort());
    const all = Object.values(parts).flatMap((p) => Object.keys(p.pt));
    expect(all.length).toBe(new Set(all).size);
    for (const k of Object.keys(pt)) {
      expect(names(en[k]!), k).toEqual(names(pt[k]!));
      expect(names(ptBR[k]!), k).toEqual(names(pt[k]!));
      expect(pt[k]!.trim(), k).not.toBe("");
      expect(en[k]!.trim(), k).not.toBe("");
      expect(ptBR[k]!.trim(), k).not.toBe("");
    }
  });

  it("codes are domain.snake_case", () => {
    for (const k of Object.keys(pt)) expect(k, k).toMatch(/^[a-z0-9_]+(\.[a-z0-9_]+)+$/);
  });

  it("the English text is not a copy of pt-PT (except short names)", () => {
    const same = Object.keys(pt).filter((k) => pt[k] === en[k] && pt[k]!.replace(/\{\w+\}/g, "").length > 14);
    expect(same).toEqual([]);
  });

  it("every key of every dictionary resolves in the three languages", () => {
    for (const l of LOCALES) {
      setLocale(l);
      for (const k of leafPaths(pt).length ? Object.keys(pt) : [])
        expect(MSG[k as keyof typeof MSG], `${l} ${k}`).toBeTruthy();
    }
    setLocale("pt-PT");
  });
});

describe("msgText / errors with a code", () => {
  it("fills params and leaves unknown names alone", () => {
    expect(fill("{a} e {b}", { a: 1 })).toBe("1 e {b}");
    expect(msgText("nope.unknown")).toBeUndefined();
    expect(coded(undefined, undefined, "English")).toBe("English");
    expect(coded("nope.unknown", {}, "English")).toBe("English");
  });

  it("a coded AppError reads in the UI language, an unknown code keeps the English text", () => {
    const k = Object.keys(pt)[0];
    if (k) {
      setLocale("en");
      const e = toAppError("x.y", { message: "daemon english", code: k, params: {} });
      expect(e).toBeInstanceOf(AppError);
      expect(e.message).toBe(fill(en[k]!, {}));
      setLocale("pt-PT");
      expect(errorText(e)).toBe(fill(pt[k]!, {}));
    }
    const u = toAppError("x.y", { message: "daemon english", code: "nope.unknown" });
    expect(u.message).toBe("daemon english");
    expect(errorText({ message: "m", code: "nope.unknown" })).toBe("m");
  });
});
