import { describe, expect, it } from "vitest";
import { pushRecent, rank } from "./paletteLogic";

const it_ = (label: string, id = label) => ({ label, id });

describe("palette ranking", () => {
  const items = [
    it_("Definições"),
    it_("Novo terminal"),
    it_("Docker"),
    it_("Mostrar painel direito"),
    it_("proj / docker-fix"),
  ];
  it("keeps everything for an empty query, in order", () => {
    expect(rank(items, "").map((i) => i.label)).toEqual(items.map((i) => i.label));
  });
  it("requires every word and ignores accents and case", () => {
    expect(rank(items, "DEFINICOES").map((i) => i.label)).toEqual(["Definições"]);
    expect(rank(items, "novo xyz")).toEqual([]);
  });
  it("puts prefix matches before inner and fuzzy ones", () => {
    expect(rank(items, "doc").map((i) => i.label)).toEqual(["Docker", "proj / docker-fix"]);
    expect(rank(items, "dkr")[0].label).toBe("Docker");
  });
  it("lifts recents; alone they order an empty query", () => {
    expect(rank(items, "", ["proj / docker-fix"])[0].label).toBe("proj / docker-fix");
    expect(rank([it_("Docker"), it_("Docker compose")], "doc", ["Docker compose"])[0].label).toBe("Docker compose");
  });
  it("pushRecent dedupes and caps", () => {
    expect(pushRecent(["a", "b", "c"], "b", 3)).toEqual(["b", "a", "c"]);
    expect(pushRecent(["a", "b", "c"], "d", 3)).toEqual(["d", "a", "b"]);
  });
});
