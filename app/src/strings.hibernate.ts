import { count, dict } from "./i18n";
import { enH } from "./strings.hibernate.en";
import { ptBRH } from "./strings.hibernate.ptbr";

/** pt-PT texts of session hibernation (Settings section, sidebar, dot tooltips, notices). */
export const ptH = {
  settings: {
    title: "Hibernação de sessões",
    note: "Um agente que só espera (por CI, por um worker, por uma resposta) pode ser posto a dormir: o processo termina e a memória fica livre, mas a linha e a conversa ficam. Acorda sozinho quando chega uma mensagem, um worker acaba ou escreves nele.",
    enabled: "Hibernar sessões paradas sozinho",
    idle: "Hibernar depois de",
    idleMin: (n: number) => (n >= 60 && n % 60 === 0 ? count(n / 60, "hora", "horas") : count(n, "minuto", "minutos")),
  },
  menu: { hibernate: "Hibernar", wake: "Acordar" },
  hibernateLabel: (name: string) => `Hibernar ${name}`,
  wakeLabel: (name: string) => `Acordar ${name}`,
  dot: {
    hibernated: "Hibernada: o processo terminou, acorda quando for preciso",
    waking: "A acordar",
    queued: "A acordar: à espera de um lugar de worker",
    since: (what: string, ago: string, mb: number) => `${what} (${ago}${mb > 0 ? `, ${mb} MB libertados` : ""})`,
  },
  notice: {
    inputDropped: (name: string) =>
      `${name} estava hibernada: acordou, mas o que escreveste perdeu-se. Escreve outra vez.`,
    wakeFailed: (name: string, why: string) => `${name} não conseguiu acordar: ${why}`,
  },
};

export const H = dict<typeof ptH>(ptH, enH, ptBRH);
