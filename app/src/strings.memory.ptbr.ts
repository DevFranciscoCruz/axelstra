import { type Widen, count, fmtFixed, fmtNumber } from "./i18n";
import type { ptG } from "./strings.memory";

/** Brazilian Portuguese texts of the resource governor (status bar chip, memory panel, Settings section, sidebar, notices). */
const size = (mb: number) => (mb >= 1024 ? `${fmtFixed(mb / 1024, 1)} GB` : `${fmtNumber(Math.round(mb))} MB`);

export const ptBRG: Widen<typeof ptG> = {
  level: { normal: "normal", high: "alta", critical: "crítica" },
  size,
  chip: {
    text: (free: string) => `${free} livres`,
    label: (level: string, free: string) => `Pressão de memória ${level}: ${free} livres. Abre o painel da memória.`,
    pausedLabel: (n: number) => `${count(n, "item pausado", "itens pausados")}. Abre o painel da memória.`,
  },
  panel: {
    title: "Memória",
    summary: (level: string, free: string, total: string) => `Pressão ${level} · ${free} livres de ${total}`,
    top: "Sessões que mais usam memória",
    topNone: "Sem dados de memória das sessões.",
    rss: (s: string) => `${s} em uso`,
    session: (id: number) => `Sessão ${id}`,
    pausedHead: "Pausados",
    pausedNone: "Nada pausado.",
    pausedMemory: "pouca memória",
    pausedManual: "manual",
    build: (name: string) => `build ${name}`,
    resume: "Retomar",
    resumeOne: (who: string) => `Retomar ${who}`,
    resumeAll: "Retomar tudo",
    configure: "Configurações da memória",
    oldDaemon: "O daemon em execução não tem o governador de recursos. Reinicie o Axelstra para usá-lo.",
  },
  breakdown: {
    head: "Para onde vai a memória",
    category: {
      sessions: "Sessões do Axelstra",
      builds: "Compilações (axelstra lock)",
      external: "Outros programas",
      kernel_paged: "Kernel (paginado)",
      kernel_nonpaged: "Kernel (não paginado)",
      cache: "Cache do sistema",
      compressed: "Memória comprimida",
      others: "Outros",
    },
    row: (size: string, pct: string) => `${size} (${pct}%)`,
    pct: (p: number) => fmtFixed(p, 1),
    cpu: (user: string, kernel: string) => `CPU: usuário ${user}%, kernel ${kernel}%`,
    commit: (used: string, limit: string) => `Commit: ${used} de ${limit}`,
    topCpuHead: "Programas que mais usam CPU",
    topIoHead: "Programas com mais E/S de disco",
    value: (name: string, v: string) => `${name}: ${v}`,
    externalHead: "Programas fora do Axelstra que mais usam",
    external: (name: string, n: number) => (n > 1 ? `${name} ×${n}` : name),
    externalNote: "Só informação: o Axelstra não mexe nesses programas.",
    handlesHead: "Programas com mais handles",
    handles: (name: string, pid: number, n: string) => `${name} (${pid}): ${n} handles`,
    kernelHigh: (size: string) =>
      `A memória do kernel está alta (${size}): possível vazamento de handles ou de um driver. Reiniciar a libera.`,
  },
  dot: {
    memory: "Pausada: pouca memória",
    manual: "Pausada (manual)",
    since: (what: string, ago: string) => `${what} · ${ago}`,
  },
  menu: {
    pause: "Pausar",
    resume: "Retomar",
  },
  sidebar: {
    rss: (s: string) => `Memória: ${s}`,
    pauseLabel: (who: string) => `Pausar ${who}`,
    resumeLabel: (who: string) => `Retomar ${who}`,
  },
  notify: {
    level: (level: "normal" | "high" | "critical", free: number): string =>
      level === "normal"
        ? "A memória voltou ao normal."
        : level === "high"
          ? `Memória alta: ${size(free)} livres.`
          : `Memória crítica: ${size(free)} livres.`,
    paused: (n: number) => `${count(n, "sessão pausada", "sessões pausadas")} por falta de memória.`,
    resumed: (n: number) => `${count(n, "sessão retomada", "sessões retomadas")}: a memória se recuperou.`,
  },
  settings: {
    title: "Governador de recursos",
    note: "Vigia a memória livre da máquina. Com pouca memória ele avisa, atrasa o início de agentes novos e pode pausar o que mais consome, para a máquina não ficar sem memória. Você retoma quando quiser, ou eles retomam sozinhos quando a memória se recupera.",
    enabled: "Ativar o governador",
    admission: "Esperar por memória antes de iniciar agentes e builds",
    autoPause: "Pausar sessões automaticamente com memória crítica",
    warnOnly: "Só avisar: nunca pausar nem atrasar nada",
    warnOnlyHint: "Útil para ver o que o governador faria sem que ele mexa em nada.",
    thresholds: "Limites de memória livre",
    thresholdsNote:
      "O nível sobe quando a memória livre cai abaixo do limite em porcentagem ou em MB. O crítico precisa ser mais baixo que o alto.",
    highPct: "Alto: livre abaixo de (%)",
    critPct: "Crítico: livre abaixo de (%)",
    highMb: "Alto: livre abaixo de (MB)",
    critMb: "Crítico: livre abaixo de (MB)",
    kernelPool: "Kernel: avisar acima de (MB)",
    kernelPoolNote: "Avisa uma vez quando o kernel (paginado + não paginado) passa desse valor.",
    kernelPoolRange: "O limite do kernel precisa estar entre 256 e 1048576 MB.",
    save: "Salvar limites",
    saved: "Limites de memória salvos.",
    notNumber: "Digite números inteiros em todos os campos de limites.",
    problem: {
      pct_range: "As porcentagens precisam estar entre 1 e 99.",
      mb_range: "Os valores em MB precisam ser maiores que zero.",
      pct_order: "O limite crítico (%) precisa ser mais baixo que o alto.",
      mb_order: "O limite crítico (MB) precisa ser mais baixo que o alto.",
    },
  },
};
