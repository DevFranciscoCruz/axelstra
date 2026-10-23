import { describe, expect, it } from "vitest";
import { ago } from "./time";

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
    expect(ago(86400 * 3)).toBe("há 3 d");
  });
});
