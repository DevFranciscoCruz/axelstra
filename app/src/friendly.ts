/** the daemon and git speak English; the common failures get a sentence that says what to do */
const FRIENDLY: [RegExp, string][] = [
  [/os error 2|program not found|not recognized|cannot find the file/i, "Programa não encontrado. Confirma que está instalado e no PATH (git, claude, codex ou gh)."],
  [/is already checked out|already exists/i, "Já existe um branch ou worktree com esse nome. Escolhe outro nome."],
  [/not a git repository/i, "Esta pasta não é um repositório git."],
  [/contains modified or untracked files|local changes would be overwritten/i, "Há alterações por guardar. Faz commit ou descarta-as primeiro."],
  [/could not read from remote|authentication failed|permission denied \(publickey\)|could not resolve host/i, "Sem acesso ao remoto. Verifica a ligação à internet e o login no git/gh."],
  [/non-fast-forward|failed to push|rejected/i, "O remoto recusou o push porque tem commits novos. Faz pull antes de enviar."],
  [/worker limit reached/i, "Limite de agentes em simultâneo atingido. Espera que um termine ou aumenta com `axelstra limit`."],
  [/connection (refused|reset)|pipe.*(closed|broken)|os error 232|os error 109/i, "Sem ligação ao daemon. Vai tentar de novo; se persistir, reinicia o Axelstra."],
  [/sem resposta do daemon/i, "O daemon não respondeu a tempo. Se persistir, reinicia o Axelstra."],
  [/unknown method/i, "O daemon em execução é antigo. Reinicia o Axelstra para o atualizar."],
];
export const friendlyError = (m: string) => {
  const hit = FRIENDLY.find(([re]) => re.test(m));
  return hit ? `${hit[1]}\n(${m.length > 160 ? m.slice(0, 160) + "…" : m})` : m;
};
