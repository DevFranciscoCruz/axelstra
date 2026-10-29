import { count, dict, fmtFixed, fmtNumber } from "./i18n";
import { enG } from "./strings.memory.en";
import { ptBRG } from "./strings.memory.ptbr";

/** pt-PT texts of the resource governor (status bar chip, memory panel, Settings section, sidebar, notices). */
export const ptG = {
  level: { normal: "normal", high: "alta", critical: "crítica" },
  /** a size in MB or GB, in the language's number format */
  size: (mb: number) => (mb >= 1024 ? `${fmtFixed(mb / 1024, 1)} GB` : `${fmtNumber(Math.round(mb))} MB`),
  chip: {
    text: (free: string) => `${free} livres`,
    label: (level: string, free: string) => `Pressão de memória ${level}: ${free} livres. Abre o painel da memória.`,
    pausedLabel: (n: number) => `${count(n, "item em pausa", "itens em pausa")}. Abre o painel da memória.`,
  },
  panel: {
    title: "Memória",
    summary: (level: string, free: string, total: string) => `Pressão ${level} · ${free} livres de ${total}`,
    top: "Sessões que mais memória usam",
    topNone: "Sem dados de memória das sessões.",
    rss: (size: string) => `${size} em uso`,
    session: (id: number) => `Sessão ${id}`,
    pausedHead: "Em pausa",
    pausedNone: "Nada em pausa.",
    pausedMemory: "pouca memória",
    pausedManual: "manual",
    build: (name: string) => `build ${name}`,
    resume: "Retomar",
    resumeOne: (who: string) => `Retomar ${who}`,
    resumeAll: "Retomar tudo",
    configure: "Definições da memória",
    oldDaemon: "O daemon em execução não tem o governador de recursos. Reinicia o Axelstra para o usar.",
  },
  dot: {
    memory: "Em pausa: pouca memória",
    manual: "Em pausa (manual)",
    since: (what: string, ago: string) => `${what} · ${ago}`,
  },
  menu: {
    pause: "Pôr em pausa",
    resume: "Retomar",
  },
  sidebar: {
    rss: (size: string) => `Memória: ${size}`,
    pauseLabel: (who: string) => `Pôr ${who} em pausa`,
    resumeLabel: (who: string) => `Retomar ${who}`,
  },
  notify: {
    level: (level: "normal" | "high" | "critical", free: number): string =>
      level === "normal"
        ? "A memória voltou ao normal."
        : level === "high"
          ? `Memória alta: ${ptG.size(free)} livres.`
          : `Memória crítica: ${ptG.size(free)} livres.`,
    paused: (n: number) => `${count(n, "sessão posta em pausa", "sessões postas em pausa")} por falta de memória.`,
    resumed: (n: number) => `${count(n, "sessão retomada", "sessões retomadas")}: a memória recuperou.`,
  },
  settings: {
    title: "Governador de recursos",
    note: "Vigia a memória livre da máquina. Com pouca memória avisa, atrasa o arranque de agentes novos e pode pôr em pausa o que mais consome, para a máquina não ficar sem memória. Retomas quando quiseres, ou sozinho quando a memória recupera.",
    enabled: "Ativar o governador",
    admission: "Esperar por memória antes de iniciar agentes e builds",
    autoPause: "Pôr em pausa sessões automaticamente com memória crítica",
    warnOnly: "Só avisar: nunca pôr em pausa nem atrasar nada",
    warnOnlyHint: "Útil para ver o que o governador faria sem ele mexer em nada.",
    thresholds: "Limites de memória livre",
    thresholdsNote:
      "O nível sobe quando a memória livre cai abaixo do limite em percentagem ou em MB. O crítico tem de ser mais baixo do que o alto.",
    highPct: "Alto: livre abaixo de (%)",
    critPct: "Crítico: livre abaixo de (%)",
    highMb: "Alto: livre abaixo de (MB)",
    critMb: "Crítico: livre abaixo de (MB)",
    save: "Guardar limites",
    saved: "Limites de memória guardados.",
    notNumber: "Escreve números inteiros em todos os campos de limites.",
    problem: {
      pct_range: "As percentagens têm de estar entre 1 e 99.",
      mb_range: "Os valores em MB têm de ser maiores do que zero.",
      pct_order: "O limite crítico (%) tem de ser mais baixo do que o alto.",
      mb_order: "O limite crítico (MB) tem de ser mais baixo do que o alto.",
    },
  },
};

export const G = dict<typeof ptG>(ptG, enG, ptBRG);
