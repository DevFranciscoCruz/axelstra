import { describe, expect, it } from "vitest";
import { waitShellReady } from "./shellReady";

const noSleep = () => Promise.resolve();

describe("waitShellReady", () => {
  it("is ready once the prompt is on screen and stable", async () => {
    const screens = [[""], ["PS C:\\w>"], ["PS C:\\w> "], ["PS C:\\w>"]];
    let i = 0;
    expect(await waitShellReady(async () => screens[Math.min(i++, screens.length - 1)], { sleep: noSleep })).toBe(true);
    expect(i).toBe(3);
  });
  it("gives up after the tries when nothing is ever printed", async () => {
    let n = 0;
    expect(await waitShellReady(async () => (n++, [""]), { tries: 5, sleep: noSleep })).toBe(false);
    expect(n).toBe(5);
  });
  it("survives a read that throws", async () => {
    let n = 0;
    const r = await waitShellReady(
      async () => {
        if (n++ < 2) throw new Error("no session");
        return ["$"];
      },
      { sleep: noSleep },
    );
    expect(r).toBe(true);
  });
});
