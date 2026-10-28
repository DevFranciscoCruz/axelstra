import type { Widen } from "../i18n";

/** Add Project sources: discover, clone, init and GitHub repositories (crates/daemon/src/server/sources.rs). */
export const pt = {
  "project.not_discovered": "{path} não foi oferecida pela deteção de repositórios. Escolhe-a outra vez na lista.",
  "project.clone_url_empty": "O endereço do repositório está vazio.",
  "project.clone_url_invalid":
    "{url} não é um endereço que o git possa clonar. Usa https://, ssh:// ou git@servidor:caminho.",
  "project.clone_branch_invalid": "{branch} não é um nome de branch válido.",
  "project.clone_depth_invalid": "A profundidade tem de ser um número inteiro de 1 a 1000000.",
  "project.clone_dest_relative": "{path} não é um caminho absoluto.",
  "project.clone_dest_exists": "{path} já existe e não está vazia.",
  "project.clone_dest_busy": "Já está a correr um clone para {path}.",
  "project.clone_too_many": "Já estão a correr {max} clones. Espera que um termine.",
  "project.clone_spawn": "Não foi possível iniciar o git clone.\n({detail})",
  "project.clone_failed": "O git clone falhou.\n({detail})",
  "project.clone_auth":
    "O git não conseguiu autenticar-se. Inicia sessão com `gh auth login` ou configura as tuas credenciais git.\n({detail})",
  "project.clone_unknown": "Não existe o clone {id}.",
  "project.init_name_invalid":
    '{name} não é um nome de pasta válido: usa um só nome, sem / \\ : * ? " < > |, sem terminar em ponto nem usar nomes reservados do Windows.',
  "project.init_parent_missing": "{path} não é uma pasta existente.",
  "project.init_exists": "{path} já existe e não está vazia.",
  "project.init_failed": "Não foi possível criar o repositório.\n({detail})",
  "project.init_no_identity":
    "O repositório foi criado, mas o git não tem nome e email para assinar o primeiro commit (git config --global user.name / user.email).",
  "gh.repos_failed": "O gh não conseguiu listar os repositórios.\n({detail})",
};

export const en: Widen<typeof pt> = {
  "project.not_discovered": "{path} was not offered by repository detection. Pick it again from the list.",
  "project.clone_url_empty": "The repository address is empty.",
  "project.clone_url_invalid": "{url} is not an address git may clone. Use https://, ssh:// or git@host:path.",
  "project.clone_branch_invalid": "{branch} is not a valid branch name.",
  "project.clone_depth_invalid": "Depth must be a whole number from 1 to 1000000.",
  "project.clone_dest_relative": "{path} is not an absolute path.",
  "project.clone_dest_exists": "{path} already exists and is not empty.",
  "project.clone_dest_busy": "A clone into {path} is already running.",
  "project.clone_too_many": "{max} clones are already running. Wait for one to finish.",
  "project.clone_spawn": "Could not start git clone.\n({detail})",
  "project.clone_failed": "git clone failed.\n({detail})",
  "project.clone_auth":
    "git could not authenticate. Sign in with `gh auth login` or set up your git credentials.\n({detail})",
  "project.clone_unknown": "There is no clone {id}.",
  "project.init_name_invalid":
    '{name} is not a valid folder name: use a single name, without / \\ : * ? " < > |, not ending in a dot or using reserved Windows names.',
  "project.init_parent_missing": "{path} is not an existing folder.",
  "project.init_exists": "{path} already exists and is not empty.",
  "project.init_failed": "Could not create the repository.\n({detail})",
  "project.init_no_identity":
    "The repository was created, but git has no name and email to sign the first commit (git config --global user.name / user.email).",
  "gh.repos_failed": "gh could not list the repositories.\n({detail})",
};

export const ptBR: Widen<typeof pt> = {
  "project.not_discovered": "{path} não foi oferecida pela detecção de repositórios. Escolha-a de novo na lista.",
  "project.clone_url_empty": "O endereço do repositório está vazio.",
  "project.clone_url_invalid":
    "{url} não é um endereço que o git possa clonar. Use https://, ssh:// ou git@servidor:caminho.",
  "project.clone_branch_invalid": "{branch} não é um nome de branch válido.",
  "project.clone_depth_invalid": "A profundidade precisa ser um número inteiro de 1 a 1000000.",
  "project.clone_dest_relative": "{path} não é um caminho absoluto.",
  "project.clone_dest_exists": "{path} já existe e não está vazia.",
  "project.clone_dest_busy": "Já tem um clone em andamento para {path}.",
  "project.clone_too_many": "Já tem {max} clones em andamento. Espere um terminar.",
  "project.clone_spawn": "Não foi possível iniciar o git clone.\n({detail})",
  "project.clone_failed": "O git clone falhou.\n({detail})",
  "project.clone_auth":
    "O git não conseguiu se autenticar. Entre com `gh auth login` ou configure suas credenciais do git.\n({detail})",
  "project.clone_unknown": "Não existe o clone {id}.",
  "project.init_name_invalid":
    '{name} não é um nome de pasta válido: use um nome só, sem / \\ : * ? " < > |, sem terminar em ponto nem usar nomes reservados do Windows.',
  "project.init_parent_missing": "{path} não é uma pasta existente.",
  "project.init_exists": "{path} já existe e não está vazia.",
  "project.init_failed": "Não foi possível criar o repositório.\n({detail})",
  "project.init_no_identity":
    "O repositório foi criado, mas o git não tem nome e e-mail para assinar o primeiro commit (git config --global user.name / user.email).",
  "gh.repos_failed": "O gh não conseguiu listar os repositórios.\n({detail})",
};
