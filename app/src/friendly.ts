import { S } from "./strings";

/** the daemon and git speak English; the common failures get a sentence that says what to do */
const FRIENDLY: [RegExp, string][] = [
  [/os error 2|program not found|not recognized|cannot find the file/i, "Programa não encontrado. Confirma que está instalado e no PATH (git, claude, codex ou gh)."],
  [/is already checked out|already exists/i, "Já existe um branch ou worktree com esse nome. Escolhe outro nome."],
  [/not a git repository/i, "Esta pasta não é um repositório git."],
  [/contains modified or untracked files|local changes would be overwritten/i, "Há alterações por guardar. Faz commit ou descarta-as primeiro."],
  [/could not read from remote|authentication failed|permission denied \(publickey\)|could not resolve host/i, "Sem acesso ao remoto. Verifica a ligação à internet e o login no git/gh."],
  [/non-fast-forward|failed to push|rejected/i, "O remoto recusou o envio porque tem commits novos. Obtém as alterações do remoto antes de enviar."],
  [/worker limit reached/i, "Limite de agentes em simultâneo atingido. Espera que um termine ou aumenta com `axelstra limit`."],
  [/connection (refused|reset)|pipe.*(closed|broken)|os error 232|os error 109/i, "Sem ligação ao daemon. Vai tentar de novo; se persistir, reinicia o Axelstra."],
  [/sem resposta do daemon/i, "O daemon não respondeu a tempo. Se persistir, reinicia o Axelstra."],
  [/unknown method/i, "O daemon em execução é antigo. Reinicia o Axelstra para o atualizar."],
  [/cannot connect to the docker daemon|docker daemon is not running|error during connect/i, "O Docker não está a correr. Abre o Docker Desktop (ou inicia o serviço) e tenta de novo."],
  [/no such container|no such image|no such network|no such volume/i, "O Docker já não tem esse recurso: foi apagado entretanto. Atualiza a lista."],
  [/port is already allocated|address already in use/i, "A porta já está a ser usada por outro programa. Para esse programa ou escolhe outra porta."],
  [/gh auth login|not logged in|http 401|bad credentials|requires authentication/i, "O gh não tem sessão iniciada. Corre `gh auth login` num terminal."],
  [/merge conflict|automatic merge failed|conflict \(/i, "Há conflitos de integração. Resolve os ficheiros em conflito e faz commit."],
  [/nothing to commit|no changes added to commit/i, "Não há alterações para fazer commit."],
  [/pathspec .* did not match|unknown revision|bad revision|not a valid object name|invalid reference/i, "O branch ou commit indicado não existe neste repositório."],
  [/rate limit/i, "O GitHub limitou os pedidos. Espera uns minutos e tenta de novo."],
];

/** what a message from git, gh or docker usually looks like: English, with their prefixes */
const TOOL_ENGLISH = /^\s*(fatal|error|warning|hint|remote|gh|docker|git)\b[:\s]|error response from daemon|\bgit\b.*\b(failed|exited)\b|\bdocker\b.*\b(failed|exited)\b|\bgh\b.*\b(failed|exited)\b/i;

const clip = (m: string) => (m.length > 160 ? m.slice(0, 160) + "…" : m);

/** the message in Portuguese when it is a known failure, with the original in brackets; an unknown message from git, gh or
 *  docker gets a Portuguese line in front, so it never shows raw English alone; anything else is left as it is */
export const friendlyError = (m: string) => {
  const hit = FRIENDLY.find(([re]) => re.test(m));
  if (hit) return `${hit[1]}\n(${clip(m)})`;
  if (TOOL_ENGLISH.test(m)) return `${S.toolFailed}\n(${clip(m)})`;
  return m;
};
