import { describe, expect, it } from "vitest";
// @ts-expect-error the app has no node typings; the test runs under node
import { readFileSync } from "node:fs";
import { contrast } from "./a11yRules";
import { THEMES } from "./termprefs";

const raw = import.meta.glob<string>(["./ui/{button,select,dropdown-menu,dialog}.tsx", "./terminal/createXterm.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
});
const file = (p: string): string => readFileSync(p, "utf8"); // css modules come empty through vitest; vitest runs from app/
const src = (name: string) => Object.entries(raw).find(([k]) => k.endsWith(name))?.[1] ?? "";

describe("focus indicators and terminal contrast", () => {
  it("Button, Select and menus draw a full-strength ring, hidden (not removed) outline", () => {
    for (const f of ["ui/button.tsx", "ui/select.tsx", "ui/dropdown-menu.tsx"]) {
      const s = src(f);
      expect(s, f).not.toMatch(/ring-ring\/\d/);
      expect(s, f).not.toMatch(/(^|[\s"'])outline-none/);
    }
  });
  it("highlighted menu and Select items carry a ring, not only a background", () => {
    expect(src("ui/dropdown-menu.tsx")).toContain("data-[highlighted]:ring-2");
    expect(src("ui/select.tsx")).toContain("data-[highlighted]:ring-2");
  });
  it("forced colours get a real outline; small targets grow to 24px", () => {
    const css = file("src/a11y.css");
    expect(css).toContain("forced-colors: active");
    expect(css).toContain("max(100%, 24px)");
    expect(file("src/tailwind.css")).toContain("a11y.css");
  });
  it("the terminal enforces a minimum contrast and the light palette passes 4.5:1", () => {
    expect(src("createXterm.ts")).toContain("minimumContrastRatio: 4.5");
    const t = THEMES.light;
    for (const [k, v] of Object.entries(t)) {
      if (k === "background" || k === "selectionBackground" || k === "cursor" || typeof v !== "string") continue;
      expect(contrast(v, t.background as string), k).toBeGreaterThanOrEqual(4.5);
    }
  });
});
