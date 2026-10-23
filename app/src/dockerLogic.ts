import type { DockerContainer, DockerImage, DockerPort } from "./api";

export type Filter = "all" | "running" | "stopped";
export type Group = { project: string; workingDir: string; items: DockerContainer[]; running: number };

const STATES: Record<string, { text: string; cls: "on" | "warn" | "off" }> = {
  running: { text: "a correr", cls: "on" },
  paused: { text: "em pausa", cls: "warn" },
  restarting: { text: "a reiniciar", cls: "warn" },
  created: { text: "criado", cls: "off" },
  exited: { text: "parado", cls: "off" },
  dead: { text: "morto", cls: "off" },
};
/** Label and colour class of a container state; the label is always shown, the colour is only a hint. */
export const stateInfo = (state: string) => STATES[state] ?? { text: state || "desconhecido", cls: "off" as const };

export const isRunning = (c: DockerContainer) => c.state === "running";

/** Search box + filter: every word has to appear in the name, image, project, service or a published port. */
export const filterContainers = (list: DockerContainer[], query: string, filter: Filter) => {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((c) => {
    if (filter === "running" && !isRunning(c)) return false;
    if (filter === "stopped" && isRunning(c)) return false;
    const hay = [c.name, c.image, c.project, c.service, ...c.ports.map((p) => String(p.host ?? p.container))].join(" ").toLowerCase();
    return words.every((w) => hay.includes(w));
  });
};

/** Compose projects first (by name), then the containers that belong to none (project ""). */
export const groupByProject = (list: DockerContainer[]): Group[] => {
  const by = new Map<string, Group>();
  for (const c of list) {
    const g = by.get(c.project) ?? { project: c.project, workingDir: c.working_dir, items: [], running: 0 };
    g.items.push(c);
    if (isRunning(c)) g.running++;
    by.set(c.project, g);
  }
  return [...by.values()].sort((a, b) => (a.project === "" ? 1 : b.project === "" ? -1 : a.project.localeCompare(b.project)));
};

export type PortChip = { label: string; url: string | null };
/** A published port opens http://localhost:PORT; a port that is only exposed is shown but not clickable. */
export const portChips = (ports: DockerPort[]): PortChip[] =>
  ports.map((p) => (p.host != null ? { label: p.host === p.container ? `${p.host}` : `${p.host}→${p.container}`, url: `http://localhost:${p.host}` } : { label: `${p.container}`, url: null }));

/** "2026-10-18 11:09:52 +0100 BST" as the instant it names */
export const parseCreated = (s: string): number | null => {
  const m = /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}) ([+-]\d{2})(\d{2})/.exec(s);
  const t = m ? Date.parse(`${m[1]}T${m[2]}${m[3]}:${m[4]}`) : Date.parse(s);
  return Number.isNaN(t) ? null : t;
};

export const ago = (ms: number | null, now = Date.now()) => {
  if (ms == null) return "";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return "agora";
  const units: [number, string, string][] = [[60, "minuto", "minutos"], [24, "hora", "horas"], [30, "dia", "dias"], [12, "mês", "meses"], [Infinity, "ano", "anos"]];
  let n = s / 60;
  for (const [step, one, many] of units) {
    if (n < step) return `há ${Math.floor(n)} ${Math.floor(n) === 1 ? one : many}`;
    n /= step;
  }
  return "";
};

/** Containers that run from this image, by tag (a bare name means :latest) or by id. */
export const imageUsers = (img: DockerImage, containers: DockerContainer[]) =>
  containers.filter((c) => {
    const ref = c.image.includes(":") || c.image.includes("@") ? c.image : `${c.image}:latest`;
    return img.tags.includes(ref) || (c.image.length >= 6 && img.id.startsWith(c.image.replace(/^sha256:/, "").slice(0, 12)));
  });

const SECRET = /(pass(word|wd)?|secret|token|api[_-]?key|private|credential|auth)/i;
/** Env var values that look like secrets start hidden in the Info tab. */
export const isSecretKey = (k: string) => SECRET.test(k);

/** Lines of a log that contain every word of the query (case-insensitive). */
export const searchLog = (text: string, query: string) => {
  const lines = text.split(/\r?\n/);
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return words.length ? lines.filter((l) => words.every((w) => l.toLowerCase().includes(w))) : lines;
};

/** "/etc" + "nginx" -> "/etc/nginx"; ".." goes up; never above "/" */
export const joinPath = (dir: string, name: string) => {
  if (name === "..") return dir.replace(/\/[^/]*\/?$/, "") || "/";
  return `${dir.replace(/\/$/, "")}/${name}`;
};
