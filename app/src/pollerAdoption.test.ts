import { describe, expect, it } from "vitest";

const files = import.meta.glob<string>(["./Sidebar.tsx", "./Kanban.tsx", "./Orchestra.tsx", "./Usage.tsx", "./LimitsBar.tsx", "./Docker.tsx"], { query: "?raw", import: "default", eager: true });

describe("F12 polling goes through createPoller", () => {
  it("no hand-rolled visibility-guarded setInterval in the polling components", () => {
    const bad = Object.entries(files).filter(([, t]) => /setInterval\(/.test(t)).map(([f]) => f);
    expect(bad).toEqual([]);
  });
});
