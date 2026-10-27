import { describe, expect, it } from "vitest";

const files = import.meta.glob<string>(["./**/*.ts", "./**/*.tsx", "!./**/*.test.ts", "!./**/*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

describe("the app is event driven: no polling", () => {
  it("no setInterval anywhere in the sources", () => {
    const bad = Object.entries(files)
      .filter(([, t]) => /\bsetInterval\s*\(/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });

  it("the old poller is gone and nobody imports it", () => {
    expect(Object.keys(files).filter((f) => /\/poller\.ts$/.test(f))).toEqual([]);
    const bad = Object.entries(files)
      .filter(([, t]) => /createPoller|from\s+["'](\.\.?\/)+poller["']/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });

  it("the panels that used to poll subscribe through createLive", () => {
    const panels = [
      "./Kanban.tsx",
      "./Orchestra.tsx",
      "./Docker.tsx",
      "./docker/Detail.tsx",
      "./LimitsBar.tsx",
      "./Usage.tsx",
      "./Sidebar.tsx",
      "./ShipPanel.tsx",
      "./store/git.ts",
      "./addProject/cloneJob.ts",
    ];
    const missing = panels.filter((f) => !/createLive\(/.test(files[f] ?? ""));
    expect(missing).toEqual([]);
  });

  it("no timer that re-arms itself in a loop", () => {
    const bad = Object.entries(files)
      .filter(([, t]) => /=\s*setTimeout\(\s*(loop|poll|tick)\b/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });
});
