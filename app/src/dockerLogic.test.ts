import { describe, expect, it } from "vitest";
import type { DockerContainer, DockerImage } from "./api";
import { ago, filterContainers, groupByProject, imageUsers, isSecretKey, joinPath, parseCreated, portChips, searchLog, stateInfo } from "./dockerLogic";

const c = (o: Partial<DockerContainer>): DockerContainer => ({
  id: "1", name: "n", image: "redis:8", state: "running", status: "Up", ports: [], created: "", project: "", working_dir: "", service: "", volumes: [], networks: [], ...o,
});

describe("grouping and filtering", () => {
  const list = [
    c({ id: "1", name: "demo-db-1", project: "demo", image: "postgres:18", ports: [{ host: 5432, container: 5432, proto: "tcp" }] }),
    c({ id: "2", name: "demo-redis-1", project: "demo", state: "exited", service: "redis" }),
    c({ id: "3", name: "web", project: "" }),
    c({ id: "4", name: "alpha-1", project: "alpha" }),
  ];
  it("puts compose projects first by name and loose containers last", () => {
    const g = groupByProject(list);
    expect(g.map((x) => x.project)).toEqual(["alpha", "demo", ""]);
    expect(g[1].running).toBe(1);
    expect(g[1].items).toHaveLength(2);
  });
  it("filters by state and by every search word", () => {
    expect(filterContainers(list, "", "stopped").map((x) => x.id)).toEqual(["2"]);
    expect(filterContainers(list, "", "running").map((x) => x.id)).toEqual(["1", "3", "4"]);
    expect(filterContainers(list, "demo postgres", "all").map((x) => x.id)).toEqual(["1"]);
    expect(filterContainers(list, "5432", "all").map((x) => x.id)).toEqual(["1"]);
    expect(filterContainers(list, "nothing", "all")).toEqual([]);
  });
});

describe("port chips", () => {
  it("makes published ports clickable and leaves exposed ones plain", () => {
    expect(portChips([{ host: 8080, container: 80, proto: "tcp" }, { host: 3000, container: 3000, proto: "tcp" }, { host: null, container: 9000, proto: "tcp" }])).toEqual([
      { label: "8080→80", url: "http://localhost:8080" },
      { label: "3000", url: "http://localhost:3000" },
      { label: "9000", url: null },
    ]);
  });
});

describe("state, time and names", () => {
  it("always has a text for the state", () => {
    expect(stateInfo("running")).toEqual({ text: "a correr", cls: "on" });
    expect(stateInfo("weird").text).toBe("weird");
  });
  it("reads the date docker prints and says how long ago", () => {
    const t = parseCreated("2026-10-18 11:09:52 +0100 BST")!;
    expect(new Date(t).toISOString()).toBe("2026-10-18T10:09:52.000Z");
    expect(ago(t, t + 5 * 86400_000)).toBe("há 5 dias");
    expect(ago(t, t + 3600_000)).toBe("há 1 hora");
    expect(ago(t, t + 10_000)).toBe("agora");
    expect(parseCreated("garbage")).toBeNull();
    expect(ago(null)).toBe("");
  });
  it("hides values that look like secrets", () => {
    expect(["POSTGRES_PASSWORD", "API_KEY", "GITHUB_TOKEN", "DB_SECRET"].every(isSecretKey)).toBe(true);
    expect(["PATH", "NODE_ENV"].some(isSecretKey)).toBe(false);
  });
});

describe("images, logs and paths", () => {
  const img = (o: Partial<DockerImage>): DockerImage => ({ id: "3811787313eb", tags: ["redis:8-alpine"], size: "", created: "", containers: 0, dangling: false, ...o });
  it("finds the containers of an image by tag, implicit latest or id", () => {
    const list = [c({ id: "a", image: "redis:8-alpine" }), c({ id: "b", image: "3811787313eb" }), c({ id: "d", image: "nginx" })];
    expect(imageUsers(img({}), list).map((x) => x.id)).toEqual(["a", "b"]);
    expect(imageUsers(img({ tags: ["nginx:latest"], id: "ffffffffffff" }), list).map((x) => x.id)).toEqual(["d"]);
  });
  it("searches log lines", () => {
    expect(searchLog("a ok\nB error\nc ERROR db", "error").length).toBe(2);
    expect(searchLog("a\nb", "")).toEqual(["a", "b"]);
    expect(searchLog("a ok\nb error db", "error db")).toEqual(["b error db"]);
  });
  it("joins and climbs container paths", () => {
    expect(joinPath("/", "etc")).toBe("/etc");
    expect(joinPath("/etc", "nginx")).toBe("/etc/nginx");
    expect(joinPath("/etc/nginx", "..")).toBe("/etc");
    expect(joinPath("/etc", "..")).toBe("/");
    expect(joinPath("/", "..")).toBe("/");
  });
});
