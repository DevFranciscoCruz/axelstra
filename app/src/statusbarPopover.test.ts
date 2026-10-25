import { describe, expect, it } from "vitest";
// @ts-expect-error the app has no node typings; the test runs under node
import { readFileSync } from "node:fs";

const css = (f: string) => (readFileSync(`src/${f}`, "utf8") as string).replace(/\/\*[\s\S]*?\*\//g, "");
/** the declarations of every rule whose selector list is exactly `sel` */
const decls = (sheet: string, sel: string) =>
  [...sheet.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => m[1].split(",").some((s) => s.trim() === sel))
    .map((m) => m[2]);

describe("status bar popovers", () => {
  it("the bar does not clip: the bell popover opens upward out of its 24px", () => {
    const all = ["styles.css", "notifications.css"].flatMap((f) => decls(css(f), ".statusbar"));
    expect(all.filter((d) => /overflow(-y)?\s*:\s*(hidden|clip|auto|scroll)/.test(d))).toEqual([]);
  });
  it("the bell popover opens above the bar and grows leftwards from the bell, which sits near the right edge", () => {
    const [d] = decls(css("notifications.css"), ".statusbar .notif-pop");
    expect(d).toMatch(/bottom\s*:/);
    expect(d).toMatch(/right\s*:\s*0/);
    expect(d).toMatch(/left\s*:\s*auto/);
  });
});
