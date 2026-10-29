import { type Widen, count } from "./i18n";
import type { ptH } from "./strings.hibernate";

/** Brazilian Portuguese texts of session hibernation (Settings section, sidebar, dot tooltips, notices). */
export const ptBRH: Widen<typeof ptH> = {
  settings: {
    title: "Hibernação de sessões",
    note: "Um agente que só está esperando (por CI, por um worker, por uma resposta) pode ser colocado para dormir: o processo termina e a memória fica livre, mas a linha e a conversa continuam. Ele acorda sozinho quando chega uma mensagem, um worker termina ou você digita nele.",
    enabled: "Hibernar sessões paradas automaticamente",
    idle: "Hibernar depois de",
    idleMin: (n: number) => (n >= 60 && n % 60 === 0 ? count(n / 60, "hora", "horas") : count(n, "minuto", "minutos")),
  },
  menu: { hibernate: "Hibernar", wake: "Acordar" },
  hibernateLabel: (name: string) => `Hibernar ${name}`,
  wakeLabel: (name: string) => `Acordar ${name}`,
  dot: {
    hibernated: "Hibernada: o processo terminou, ela acorda quando for preciso",
    waking: "Acordando",
    queued: "Acordando: esperando uma vaga de worker",
    since: (what: string, ago: string, mb: number) => `${what} (${ago}${mb > 0 ? `, ${mb} MB liberados` : ""})`,
  },
  notice: {
    inputDropped: (name: string) =>
      `${name} estava hibernada: ela acordou, mas o que você digitou se perdeu. Digite de novo.`,
    wakeFailed: (name: string, why: string) => `${name} não conseguiu acordar: ${why}`,
  },
};
