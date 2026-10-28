import type { Widen } from "../i18n";

/** Texts of the message codes raised by session hibernation (prefix `hibernate.`; see crates/daemon/src/server/hibernate.rs). */
export const pt = {
  "hibernate.not_agent": "Só as sessões de agentes podem hibernar.",
  "hibernate.main": "A sessão principal do projeto nunca hiberna.",
  "hibernate.no_resume": "{agent} não consegue retomar esta conversa: não tem suporte para isso ou ainda não há conversa guardada.",
  "hibernate.ambiguous": "Outra sessão de {agent} usa esta pasta: retomar podia escolher a conversa errada.",
  "hibernate.recent": "Recebeu escrita ou novidades há poucos segundos.",
  "hibernate.working": "O agente está a trabalhar.",
  "hibernate.waiting": "O agente espera por uma resposta ou uma permissão.",
  "hibernate.command": "Há um comando, uma compilação ou um hook a correr na sessão.",
  "hibernate.event": "Chegou algo à sessão entretanto: fica acordada.",
  "hibernate.sleeping": "A sessão {id} está hibernada: acorda-a com `axelstra wake {id}`.",
  "hibernate.wake_failed": "{label} não pôde ser retomada depois de hibernar: {detail}",
  "settings.bad_hibernate": "A hibernação aceita enabled (true ou false) e idle_minutes (um número inteiro de 1 a 1440).",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "hibernate.not_agent": "Only agent sessions can hibernate.",
  "hibernate.main": "The project's main session never hibernates.",
  "hibernate.no_resume": "{agent} cannot resume this conversation: it has no support for it or there is no saved conversation yet.",
  "hibernate.ambiguous": "Another {agent} session uses this folder: resuming could pick the wrong conversation.",
  "hibernate.recent": "It got input or news a few seconds ago.",
  "hibernate.working": "The agent is working.",
  "hibernate.waiting": "The agent is waiting for an answer or a permission.",
  "hibernate.command": "A command, a build or a hook is running in the session.",
  "hibernate.event": "Something reached the session meanwhile: it stays awake.",
  "hibernate.sleeping": "Session {id} is hibernated: wake it with `axelstra wake {id}`.",
  "hibernate.wake_failed": "{label} could not be resumed after hibernating: {detail}",
  "settings.bad_hibernate": "Hibernation takes enabled (true or false) and idle_minutes (a whole number from 1 to 1440).",
};

export const ptBR: Widen<typeof pt> = {
  "hibernate.not_agent": "Só as sessões de agentes podem hibernar.",
  "hibernate.main": "A sessão principal do projeto nunca hiberna.",
  "hibernate.no_resume": "O {agent} não consegue retomar esta conversa: não tem suporte para isso ou ainda não há conversa salva.",
  "hibernate.ambiguous": "Outra sessão do {agent} usa esta pasta: retomar poderia escolher a conversa errada.",
  "hibernate.recent": "Recebeu digitação ou novidades há poucos segundos.",
  "hibernate.working": "O agente está trabalhando.",
  "hibernate.waiting": "O agente está esperando uma resposta ou uma permissão.",
  "hibernate.command": "Há um comando, uma compilação ou um hook rodando na sessão.",
  "hibernate.event": "Chegou algo à sessão nesse meio-tempo: ela fica acordada.",
  "hibernate.sleeping": "A sessão {id} está hibernada: acorde-a com `axelstra wake {id}`.",
  "hibernate.wake_failed": "{label} não pôde ser retomada depois de hibernar: {detail}",
  "settings.bad_hibernate": "A hibernação aceita enabled (true ou false) e idle_minutes (um número inteiro de 1 a 1440).",
};
