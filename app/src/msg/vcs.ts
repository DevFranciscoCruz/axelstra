import type { Widen } from "../i18n";

/** Texts of the message codes raised by the daemon (see crates/core/src/msg.rs). `{name}` is filled from the call's params.
 *  pt-PT is the source; `en` and `ptBR` must have exactly the same keys (compile error otherwise, `msg.test.ts` repeats it). */
export const pt = {
  "git.timeout": "O git {cmd} demorou mais de {secs} s e foi interrompido.",
  "git.too_big": "O git {cmd} devolveu mais de {mib} MiB, o que é demasiado.",
  "git.failed": "O git {cmd} falhou.\n({detail})",
  "git.not_repo": "Esta pasta não é um repositório git.\n({detail})",
  "git.dirty": "Há alterações por guardar. Faz commit ou descarta-as primeiro.\n({detail})",
  "git.exists": "Já existe um branch ou worktree com esse nome. Escolhe outro nome.\n({detail})",
  "git.remote": "Sem acesso ao remoto. Verifica a ligação à internet e o login no git/gh.\n({detail})",
  "git.rejected":
    "O remoto recusou o envio porque tem commits novos. Obtém as alterações do remoto antes de enviar.\n({detail})",
  "git.conflicts": "Há conflitos de merge. Resolve-os nos ficheiros e continua.\n({detail})",
  "git.nothing_to_commit": "Não há nada para fazer commit.\n({detail})",
  "git.bad_ref": "Branch, tag ou commit desconhecido. Confirma o nome.\n({detail})",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "git.timeout": "git {cmd} took more than {secs} s and was stopped.",
  "git.too_big": "git {cmd} printed more than {mib} MiB, which is too much.",
  "git.failed": "git {cmd} failed.\n({detail})",
  "git.not_repo": "This folder is not a git repository.\n({detail})",
  "git.dirty": "There are uncommitted changes. Commit or discard them first.\n({detail})",
  "git.exists": "A branch or worktree with that name already exists. Pick another name.\n({detail})",
  "git.remote": "Cannot reach the remote. Check your internet connection and your git/gh login.\n({detail})",
  "git.rejected":
    "The remote refused the push because it has newer commits. Fetch its changes before pushing.\n({detail})",
  "git.conflicts": "There are merge conflicts. Resolve them in the files and continue.\n({detail})",
  "git.nothing_to_commit": "There is nothing to commit.\n({detail})",
  "git.bad_ref": "Unknown branch, tag or commit. Check the name.\n({detail})",
};

/** Brazilian wording: written out in full, not a fallback to pt-PT. */
export const ptBR: Widen<typeof pt> = {
  "git.timeout": "O git {cmd} demorou mais de {secs} s e foi interrompido.",
  "git.too_big": "O git {cmd} devolveu mais de {mib} MiB, o que é demais.",
  "git.failed": "O git {cmd} falhou.\n({detail})",
  "git.not_repo": "Esta pasta não é um repositório git.\n({detail})",
  "git.dirty": "Há alterações não salvas. Faça commit ou descarte-as primeiro.\n({detail})",
  "git.exists": "Já existe um branch ou worktree com esse nome. Escolha outro nome.\n({detail})",
  "git.remote": "Sem acesso ao remoto. Verifique a conexão com a internet e o login no git/gh.\n({detail})",
  "git.rejected":
    "O remoto recusou o envio porque tem commits novos. Busque as alterações do remoto antes de enviar.\n({detail})",
  "git.conflicts": "Há conflitos de merge. Resolva-os nos arquivos e continue.\n({detail})",
  "git.nothing_to_commit": "Não há nada para commitar.\n({detail})",
  "git.bad_ref": "Branch, tag ou commit desconhecido. Confira o nome.\n({detail})",
};
