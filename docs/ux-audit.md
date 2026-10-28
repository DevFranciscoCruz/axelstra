# Auditoria UI/UX da app Axelstra

Auditoria **só de leitura** (nenhum ficheiro de código foi alterado), feita por 5 workers, um por área, com a skill `ui-ux-pro-max` como guia.

## Método e limites

- **A app não foi corrida em nenhuma área** (sem binário/`dist` utilizável no worktree; `tools/e2e.mjs` não chegou a arrancar). Não há screenshots. Todos os achados vêm de leitura de código com ficheiro:linha, por isso a confiança é no máximo média/alta onde o código é inequívoco (tokens, estruturas) e média/baixa onde depende de renderização real (contraste efetivo, layout em janelas estreitas, animações).
- Nenhum achado Crítico encontrado. Contagem: Alto 13 (A11y 3, Visual 3, Fluxos 2, Estados/i18n 2, Sidebar/terminais 3), Médio 37, Baixo 36.
- Cada achado traz: local, evidência, impacto, correção sugerida, esforço (S/M/L) e confiança. Ordenados por severidade dentro de cada área.
- Próximo passo recomendado: construir app + `app/dist` e validar visualmente os itens de confiança média/baixa (sobretudo A2/A3 da área 5 e A1 da área 1).

## Top 10 global (Alto, por impacto/esforço)

1. **Integrar sem confirmação, pré-visualização nem desfazer** (Fluxos A1) — ação com efeitos no ramo principal.
2. **Ecrã de primeira execução com daemon offline / projetos a carregar** (Estados A1) — a app parece uma instalação nova e induz a re-adicionar projetos.
3. **Rodapé da sidebar (Definições, Dispatcher, Automatizações) rola com a lista** (Sidebar A1).
4. **Pega de redimensionar da sidebar desalinhada em janelas estreitas** (Sidebar A2).
5. **Tab ativo pode ficar fora de vista (scroll horizontal escondido)** (Sidebar A3).
6. **Orquestração (Kanban) não se pode parar; paralelismo fixo em 3** (Fluxos A2).
7. **Seletor de agente sem estado "nenhum agente instalado"** (Estados A2).
8. **Terminal sem `minimumContrastRatio`**: texto ANSI "dim" abaixo de 3:1 (A11y A1).
9. **Anel de foco fraco em `Button`/`Select`, sem fallback em contraste forçado; item realçado em menus a 1.1–1.2:1** (A11y A2/A3).
10. **Dois sistemas de componentes e ícones em paralelo (CSS legacy vs `ui/`, ~21 SVG inline)** (Visual A2/A3) + sombras hardcoded iguais nos dois temas (A1).

Rápidos de alto retorno (esforço S): Type-to-confirm sem foco no campo (Fluxos M1), validação do nome em "Novo worktree" (M4), confirmação em "Restaurar backup"/"Limpar agora" (M5), feedback do "Copiar diagnóstico" (Estados M4), atalhos "Ctrl" fixos que não passam por `K()` (Estados M8), prioridade do dot "à espera" sobre "stalled" (Sidebar M2).


---

## Área 1 — Acessibilidade (auditoria só de leitura)

### Método e limites

- **A app NÃO correu.** O único `axelstra-app.exe` disponível (`..\axelstra\target\debug\`, de 20 Out, 8 dias antes do HEAD `f116744`) é um build debug sem `custom-protocol`. `tools/visual.mjs` terminou com `timeout waiting for the UI`, e o `app/dist` não existe no worktree. Não fiz build: o worktree não tem `node_modules` e o build é pesado. **Não há screenshots.** Os alvos de toque abaixo são calculados a partir do CSS, não medidos.
- Os rácios WCAG vêm de um script meu, fora do repo, sobre os tokens de `app/src/styles.css:1-113`. Não tocei em nenhum ficheiro do repo.
- A base já é boa: modais com `inert` no shell e focus trap, `press()`, regiões vivas, separadores com teclado, reduced-motion global, glifos nos StateDot, o `a11yRules.test.ts` e o `a11yLint.test.ts`. Por isso os achados são lacunas finas. Não encontrei nenhum Crítico.
- **Confiança global média/baixa**, porque é só código. Cada achado indica a sua.

#### Rácios dos tokens (calculados)

| Par | Escuro | Claro |
|---|---|---|
| `--text` / `--panel` | 10.80 | 15.80 |
| `--muted` / `--panel` | 5.79 | 6.27 |
| `--muted` / `--s2` | 4.53 | 4.84 |
| `--accent` / `--panel` | 5.56 | 5.46 |
| `--accent` / `--s2` | 4.35 | **4.21** |
| `--ok2` / `--ok-bg` | 6.46 | **4.40** |
| `--ok` / `--s2` | 8.2 (s3) | **3.92** |
| `--danger` / `--err-bg` | n/a | **4.43** |
| `--err` / `--panel` | **3.86** | 5.36 |
| `--err` / `--s3` (fundo do modal) | **3.64** | n/a |
| `--line-ctl` / `--s3` | **2.90** | **2.82** |
| `--line-ctl` / `--panel` | 3.08 | 3.19 |
| `--line` / `--panel` (borda de textarea/select) | **1.25** | **1.37** |
| `--s1` / `--panel` (realce de item) | 1.11 | 1.20 |

### Alto

#### A1. Terminal sem `minimumContrastRatio`: texto ANSI "dim" e de cor fica abaixo de 3:1
- **Local:** `app/src/termprefs.ts:14` (o tema escuro só define bg/fg/cursor/seleção, por isso usa a paleta ANSI por omissão do xterm). `app/src/termprefs.ts:15-36` (tema claro). `app/src/terminal/createXterm.ts:70-80` (não há `minimumContrastRatio`; `grep` não encontra nada em `app/src`). Ecrã: o terminal do agente, que é a superfície principal.
- **Evidência:** sem screenshot (a app não correu).
  - Tema claro, calculado a partir do ficheiro: `white #6e7781` 4.36, `brightWhite #8c959f` **2.91**, `brightCyan #3192aa` **3.46**, todos sobre `#fafafa`.
  - Tema escuro, assumindo a paleta xterm.js 5.5 por omissão (não verifiquei no `node_modules`): `brightBlack #666666` **2.87**, `blue #2472c8` 3.38, `red #cd3131` 3.20, `magenta #bc3fbc` 3.59 sobre `#1e1f23`. O `brightBlack` é o cinzento que as CLIs de agentes usam para texto secundário.
- **Impacto:** texto dim, diffs e prompts dos agentes ficam ilegíveis para baixa visão, e em ecrãs com brilho baixo para toda a gente.
- **Correção:** `minimumContrastRatio: 4.5` nas opções do `Terminal`. Alternativa: definir a paleta ANSI completa também no tema escuro e corrigir `brightWhite` e `brightCyan` no claro. Opcionalmente, um modo "alto contraste" nas preferências do terminal (`termprefs.ts:6`).
- **Esforço:** S. **Confiança:** média (valores do tema claro certos; paleta escura por omissão assumida).

#### A2. Anel de foco de `Button` e `Select` fraco, e `outline-none` do Tailwind 4 sem fallback em modo de contraste forçado
- **Local:** `app/src/ui/button.tsx:6` (`outline-none focus-visible:ring-2 focus-visible:ring-ring/60`). `app/src/ui/select.tsx:101` (`focus-visible:ring-2 focus-visible:ring-ring/40`, a borda não muda). Contraste com o global em `styles.css:722` (`:focus-visible` com anel interior `--ring` cheio e outline transparente).
- **Evidência (cálculo):** o anel a 60% de `--accent` dá **2.92:1** sobre o painel escuro e **2.56** sobre o branco. O de `Select` (40%) dá **2.03** e **1.82**. Ambos ficam abaixo do mínimo de 3:1 (WCAG 1.4.11 e 2.4.11). Os utilitários Tailwind estão na layer `utilities`, que vence a layer `legacy` onde está o anel global, por isso o anel forte global não se aplica a estes botões. No Tailwind 4, `outline-none` é `outline-style:none` (o equivalente a `transparent` é `outline-hidden`), por isso em Windows High Contrast (`forced-colors`, que remove `box-shadow`) estes controlos ficam sem indicador. `Input` está bem (`focus-visible:border-ring`, borda cheia). `Tabs` também (`ui/tabs.tsx:23` usa outline cheio).
- **Impacto:** quase todos os botões da app (toolbar, diálogos, toasts, painéis) e os Selects têm foco difícil de ver. Em modo claro é pior. Utilizadores de teclado e baixa visão perdem a posição.
- **Correção:** trocar por `outline-hidden focus-visible:ring-2 focus-visible:ring-ring ring-inset` (cheio, interior, como `--ring`), ou apagar o ring do Tailwind e deixar o global agir. Para `Select`, usar também `focus-visible:border-ring`. Acrescentar `@media (forced-colors: active) { :focus-visible { outline: 2px solid Highlight } }`. Um teste que valide o rácio do ring.
- **Esforço:** S. **Confiança:** média (não vi no ecrã; depende da ordem de camadas do build).

#### A3. Item realçado nos menus e Selects distingue-se do fundo só por 1.1–1.2:1
- **Local:** `app/src/ui/dropdown-menu.tsx:23` (`outline-none data-[highlighted]:bg-accent data-[expanded]:bg-accent`). `app/src/ui/select.tsx:71` (mesmo padrão). `--color-accent: var(--s1)` em `tailwind.css`. Ecrã: menus de contexto, menus "mais ações", Selects (agente, idioma, etc.).
- **Evidência:** `--s1` sobre `--panel` dá **1.11** no escuro e **1.20** (`#e8eaf0` sobre `#fff`) no claro. Não há outline, anel nem marcador no item.
- **Impacto:** quem navega com setas não vê qual item está ativo (WCAG 2.4.7 e 1.4.11). Os menus são usados por todas as ações de teclado (`menuKey`, `ContextMenu.tsx`).
- **Correção:** acrescentar ao realçado `shadow-[inset_2px_0_0_var(--accent)]` ou `ring-2 ring-inset ring-ring`, e/ou `bg-secondary` (s2, 1.28, ainda pouco) mais um marcador. Preferível anel interior, igual ao `--ring`.
- **Esforço:** S. **Confiança:** média-alta (cálculo certo; o comportamento visual não foi visto).

### Médio

#### M1. Texto de erro com `--err` abaixo de 4.5:1 no tema escuro (15 usos)
- **Local:** `text-[var(--err)]` em `addProject/CloneTab.tsx:67,92,119`, `CloneProgress.tsx:32`, `DiscoverTab.tsx:60`, `NewTab.tsx`, `GithubTab.tsx`, `FolderField.tsx`, `AddRemoteProject.tsx:231,274`, `AddHostForm.tsx:77`, `HostsSection.tsx`, `HostBadge.tsx`, `Usage.tsx`, `ui/badge.tsx`. Além disso `styles.css:774` (`.tabbar .add.rec`) e `styles.css:807` (`.computer-on`, 11px, na barra de estado).
- **Evidência:** `--err #e5484d` dá **3.64** sobre `--s3` (fundo do modal), **3.86** sobre `--panel` e **3.54** sobre `--err-bg`, tudo em tema escuro. O `--danger` já é 5.5 e existe como alternativa. O `a11yRules.test.ts:60` só testa `--ok/--warn/--danger`.
- **Impacto:** as mensagens de validação e de falha dos formulários de "Adicionar projeto" e "Adicionar host" (11–13px) são difíceis de ler no tema escuro, que é o tema por omissão.
- **Correção:** usar `--danger` para texto (ou clarear `--err` no escuro para ≥4.5 sobre `--s3`, por exemplo `#ff6b70`) e juntar `--err`, `--accent`, `--ok2` à lista do teste.
- **Esforço:** S. **Confiança:** alta (cálculo), média no impacto visual.

#### M2. Pares do tema claro entre 3.9 e 4.5:1
- **Local e valores:**
  - `.krun` (`styles.css:549`): `--ok2` sobre `--ok-bg` = **4.40**, 12px.
  - `.count` / `.item .count` (`styles.css:305,865`) sobre `.item.active` (`--s2`): `--ok` = **3.92**. O mesmo `--ok` ou `--ok2` sobre `--s2` dá 3.92.
  - Texto `--accent` sobre `--s2` = **4.21** (`.pm-row .pill.active`, `styles.css:825`, e `.item.active`).
  - `--danger` sobre `--s1` = 4.45 e sobre `--err-bg` = 4.43.
- **Impacto:** contadores e avisos curtos ficam marginalmente ilegíveis no tema claro, em linhas ativas ou realçadas.
- **Correção:** escurecer `--ok`/`--ok2` do claro um passo (por exemplo `#17692e`) e `--accent` para ≥4.6 sobre `--s2`. Alargar o teste a todos os fundos (`--s1`, `--s2`, `--s3`, `--ok-bg`, `--err-bg`) em ambos os temas.
- **Esforço:** S. **Confiança:** alta (cálculo).

#### M3. Controlos interativos aninhados dentro de `role="button"` e `role="option"`
- **Local:**
  - `Kanban.tsx:477` (cartão com `role="button" tabindex="0"`) contém `Kanban.tsx:505-510` (grupo com três `Button`: abrir worktree, repetir, abrir PR).
  - `docker/Lists.tsx:121` (`role="option"`) contém botões de portas (`Lists.tsx:~140`) e `.dk-acts role="group"` com botões de iniciar, parar, etc.
  - `Docker.tsx:333` (`role="listbox"`) contém também os `Button` de grupo (`Docker.tsx:343`).
- **Impacto:** os filhos de `button` e `option` são "apresentacionais" para o ARIA, por isso um leitor de ecrã pode ignorar ou fundir as ações. O nome acessível do cartão ou da linha junta título, corpo e chips. O padrão ARIA correto é grelha ou lista com células focáveis.
- **Correção:** cartão como `article` (ou `li`) com o título como botão de abrir e as ações como botões irmãos. No Docker, trocar `listbox/option` por `grid/row` (ou tabela com `aria-selected`), ou tirar as ações da linha e mantê-las só no menu e no painel de detalhe.
- **Esforço:** M. **Confiança:** alta (estrutura no código).

#### M4. Reordenar separadores só com o rato (WCAG 2.5.7)
- **Local:** `TabBar.tsx:74-95` (`draggable`, `onDrop` → `moveTab`). `grep` não encontra outro uso de `moveTab` (só `store/tabs.ts:46`) nem atalho de teclado.
- **Impacto:** quem não usa rato não consegue reordenar abas. O Kanban já tem alternativa por teclado (Espaço, setas, Espaço), por isso o padrão existe.
- **Correção:** `Alt+←/→` (ou Ctrl+Shift+PgUp/PgDn) sobre o separador focado, mais "Mover para a esquerda/direita" no menu de contexto da aba.
- **Esforço:** S. **Confiança:** alta.

#### M5. Botões de comentário do diff: um tab stop por linha, todos com o mesmo nome
- **Local:** `DiffView.tsx:111` (`aria-label={P.diff.commentTip}` = "Comentar e enviar ao agente", `strings.panels.ts:688`, glifo "＋"). A tabela (`DiffView.tsx:102`) não tem `aria-label` nem cabeçalhos.
- **Impacto:** num diff de centenas de linhas, Tab percorre um botão por linha (a paginação limita, mas continua a ser muita), todos anunciados de forma idêntica, sem o número da linha. O `+`/`−` está no texto (`DiffView.tsx:119`), o que é bom: a cor não é o único canal.
- **Correção:** `aria-label` com o número da linha ("Comentar linha 42"), `tabindex=-1` nos botões com abertura por tecla (por exemplo `c` na linha focada), `aria-label` e `<caption>` na tabela.
- **Esforço:** M. **Confiança:** alta.

#### M6. Alvos de clique abaixo de 24×24 px (WCAG 2.5.8)
- **Local (calculado do CSS, não medido):**
  - `.x` e `button.icon` (`styles.css:218-226,289-297`, usados em `sidebar/WorktreeRow.tsx:218,233,295` e `sidebar/ProjectHead.tsx:62,74`): ícone SVG de 13px com padding por omissão do `button` (1px 6px) dá cerca de 25×15 / 25×17.
  - Fechar aba, `Tab.tsx:~80` (`p-0.5`, ícone 13px) dá cerca de 17×17.
  - Itens da barra de estado, `size="xs"` = `h-[18px]` (`ui/button.tsx:38`, `styles.css:785`), 10 ocorrências em `StatusBar.tsx`.
  - `.cbtn` do diff (`px-0.5 text-xs`).
- **Impacto:** difícil de acertar com rato impreciso, tremor ou ecrã tátil (Windows em tablets). Os ícones de remover estão ao lado de itens clicáveis, e um erro apaga trabalho.
- **Correção:** `min-width/min-height: 24px` (ou padding) com área clicável a expandir por pseudo-elemento, sem alterar o aspeto. Manter a separação entre botões de ação destrutiva.
- **Esforço:** M. **Confiança:** média (por cálculo; medir com `getBoundingClientRect` quando a app correr).

#### M7. Diálogo de confirmação não liga a mensagem ao diálogo
- **Local:** `Confirm.tsx:109-111` (a mensagem é um `div.auto-note` solto). `ui/dialog.tsx:96-103` só dá `aria-label` (o título) e `aria-modal`, sem `aria-describedby` nem `role="alertdialog"`.
- **Impacto:** ao abrir uma confirmação destrutiva ("Remover tudo", "Descartar"), o foco vai para o botão Cancelar (`Confirm.tsx:130`) e o leitor de ecrã anuncia só "diálogo, título, botão Cancelar". A pergunta e o aviso de risco não são lidos.
- **Correção:** `Dialog` aceitar `describedBy`/`role` e o `Confirm` passar o id da mensagem, com `role="alertdialog"` quando `kind` for `danger`/`warning`. Aplicar ao `RemoveProject.tsx` e `CloseConfirm.tsx`.
- **Esforço:** S. **Confiança:** média (Kobalte não liga descrições sem `Dialog.Description`).

### Baixo

#### B1. `.kcard:hover` anula o anel de foco
`styles.css:561` define `box-shadow` em `.kcard:hover` (especificidade 0,2,0), que vence o `:focus-visible` global de `styles.css:722` (0,1,0). Com o rato parado sobre um cartão focado por teclado, o anel desaparece. Correção: `.kcard:hover:not(:focus-visible)` ou combinar a sombra com o anel. S, confiança alta.

#### B2. Indicador de foco do terminal é de 1px e 1.7:1
`styles.css:724` (`.term-host:focus-within` com `--line-3`). O cursor do xterm também sinaliza foco, mas um painel dividido (`SplitPane`) não mostra claramente qual terminal está ativo. Usar `--ring` interior, ou `--accent` a 1px. S, confiança média.

#### B3. Contorno de campos com `--line` (1.25:1) e tokens `--line-ctl` abaixo de 3:1 no modal
`styles.css:379-388` (`.rtools select`), `650` (`.sc-commit textarea`, 1px `--line`), `771` (`.voice-text`, `--line-2`), e `tailwind.css` onde `--color-input = --line-ctl` é 2.90 / 2.82 sobre `--s3`. O comentário do token (`styles.css:8`) diz 3:1; o teste (`a11yRules.test.ts:81`) usa 2.8 como limiar. Para campos de texto, a fronteira só precisa de ser percetível quando o fundo é igual ao do contentor (WCAG 1.4.11); a textarea de commit tem fundo `--bg` num painel `--panel`. Subir `--line-ctl` para ≥3:1 sobre `--s3` (escuro `#737891`, claro `#80869a`) e usá-lo nestes campos. S, confiança média.

#### B4. Textos fixos fora do i18n, ouvidos pelo leitor de ecrã
`Palette.tsx:51` (`hint: "worktree"`), `Tab.tsx:71-72` (`?? "Fechar"`), `StatusBar.tsx:324` (`title={K("Atalhos de teclado (?)")}`) ficam em pt-PT em en e pt-BR. Passar por `S.*`. S, confiança alta.

#### B5. Regiões vivas dos toasts não repetem texto igual
`ErrorToast.tsx:53-58`: duas regiões `sr-only` cujo conteúdo é `latestText(...)`. Um segundo erro com o mesmo texto não muda o nó e não é anunciado de novo. O `key` do toast reinicia o temporizador, mas o texto não varia. Alternar um sufixo invisível ou recriar o nó. S, confiança média.

#### B6. Paleta de comandos sem anúncio da contagem de resultados
`Palette.tsx:95-125`: o `combobox` usa `aria-activedescendant` (bom), mas não há região viva "N resultados". `aria-expanded="true"` fixo. S, confiança média.

#### B7. `FOCUSABLE` do focus trap incompleto
`ui/dialog.tsx:7` não inclui `a[href]`, `[contenteditable]`, `summary`, e `[tabindex]:not([tabindex="-1"])` apanha elementos `display:none`/`visibility:hidden`. Se o primeiro ou último elemento da lista estiver oculto, o salto Shift+Tab/Tab pode escapar. Filtrar por `offsetParent`/`checkVisibility()`. S, confiança baixa (não reproduzido).

#### B8. Toasts com ação de "Desfazer" fecham ao fim de 9 s
`toasts.ts:~26` (`NOTICE_TTL_MS = 9000`). Pausa com hover e foco (bom, WCAG 2.2.1), mas um utilizador de teclado ou leitor de ecrã não tem atalho para focar o toast. Acrescentar um atalho que foque o toast mais recente, ou manter toasts com ação até serem fechados. S, confiança média.

### Verificado e sem problema

- `prefers-reduced-motion` global (`styles.css:930-934`), mais `cursorBlinks()` (`termprefs.ts:57`). Os spinners dos StateDot continuam a girar (`styles.css:916`), mas entram na regra global (duração 0.001 ms).
- Modais: shell `inert`, pilha de modais, Esc só no topo, foco devolvido (`ui/dialog.tsx`, `modalStack.ts`).
- Estados nunca só por cor: StateDot com glifo e `aria-label` (`StateDot.tsx`), diff com `+`/`−`, `ship-mark`.
- Separadores de redimensionar com `role="separator"`, valores e teclado (`App.tsx:325-346`, `SplitPane.tsx:19`).
- Kanban com alternativa de teclado e região viva (`Kanban.tsx:446,477`).
- `html lang` atualizado com o idioma (`i18n.ts:33`), `screenReaderMode` do terminal opcional (`termprefs.ts:5`).

### Top 5 da área

1. **A1**: terminal sem `minimumContrastRatio` (S, uma linha em `createXterm.ts`; muito impacto).
2. **A2**: anel de foco de `Button`/`Select` abaixo de 3:1 e sem fallback em `forced-colors`.
3. **A3**: item realçado de menus/Selects a 1.1:1, sem marcador.
4. **M1**: `--err` como texto de erro, 3.6–3.9:1 no escuro (15 usos).
5. **M3**: botões dentro de `role=button`/`role=option` (Kanban, Docker).

### Ideias de melhoria

- Um teste de contraste "matriz": todos os tokens de texto contra todos os fundos (`--bg`, `--panel`, `--s1..s3`, `--*-bg`) nos dois temas, que falhe abaixo de 4.5 (texto) e 3 (bordas, anéis). Evita regressões como M1 e M2.
- Um teste que verifique o anel de foco computado (`getComputedStyle`) dos componentes da `ui/` contra o fundo, via o `e2e.mjs` quando a app correr.
- Um `@media (forced-colors: active)` central: `outline` visível em `:focus-visible`, bordas `ButtonText` em chips, cartões e linhas selecionadas (`.dk-row.sel`, `.pal-item.on`, `.item.active` hoje só se distinguem por fundo).
- Um atalho "ir para" os blocos principais (barra lateral, abas, terminal, painel direito), tipo F6, com `landmarks` (`nav`, `main`, `aside`) em `App.tsx`. Hoje só o Docker tem `region`/`nav`.
- Um modo "alto contraste" nas Definições que force tokens mais fortes (`--muted`, `--line-ctl`, anel a 3px) e o `minimumContrastRatio` 7.
- Correr esta auditoria com a app a sério (build `custom-protocol` e `tools/visual.mjs`) para medir alvos e capturar o foco em modo claro e escuro.

---

## Área 2 — Consistência visual e design system

**Método:** auditoria só por código (grep com contagens em `app/src/*.css`, `*.tsx`, `ui/`). **A app NÃO foi corrida** → sem screenshots; confiança global média/alta (contagens são factuais, impacto visual não foi observado). Nenhum ficheiro do repo foi tocado.

**Base positiva:** hex só em `:root` (styles.css:1-106) e `uiRules.test.ts` guarda hex/rgba/font-size/border-radius em CSS. 0 `<button>/<select>/<input>` crus em TSX (230 `<Button>`). Fontes e raios em CSS 100% tokenizados (excepto 3× `50%`). Os problemas estão onde os testes não olham: sombras, Tailwind arbitrário, ícones SVG, espaçamento, aliases de cor.

### Crítico
Nenhum.

### Alto

#### A1. Sombras 100% hardcoded, duplicadas e iguais nos dois temas
- Local: styles.css:560,561,570,590,601,640,701,731,790,939,974; notifications.css:3; termui.css:15 (17 `box-shadow` com `rgba(0,0,0,…)`, **14 combinações distintas**: blur 2/8/12/16/20/24/28/32/36/48 px, alfa .35/.4/.45/.55/.6). Não existe token `--shadow-*`. `--r-*`/`--fs-*`/`--sp-*` existem, sombras não. Ex.: popovers `0 8px 24px` aparecem com alfa .4, .45 e .55 (styles.css:590,701,790,570).
- Impacto: popovers/menus/diálogos com profundidade inconsistente; no tema claro a sombra preta de alfa .45-.6 fica pesada/suja (só `--ring` varia por tema). `uiRules.test` permite-o explicitamente (preto dos shadows).
- Correção: `--shadow-sm/md/lg` em `:root` e `[data-theme=light]` (alfa menor no claro); substituir os 17. Teste a exigir `var(--shadow-`.
- Esforço S · Confiança alta

#### A2. Ícones: sistema `Icon.tsx` ignorado por ~21 SVG inline com tamanhos e traços diferentes
- Local: `Icon.tsx` (46 ícones, 97 usos) vs SVG inline: StatusBar.tsx:172,179,187,196,213,319; App.tsx:432,437,442; LimitsBar.tsx:25,30; FilesPanel.tsx:63(+3); Voice.tsx:180; UpdateToast.tsx:88; StateDot.tsx:25; docker/shared.tsx:22; ui/copy-word.tsx(2), ui/checkbox-field.tsx.
- Evidência: `stroke-width` inline: 1.2(2), 1.3(3), 1.4(8), 1.5(1), 1.6(3), 2(1), 2.2(1) — 7 espessuras. Larguras fixas 8/10/12/14/16(×9). `<Icon size=…>`: 10(6), 11(22), 12(12), 13(13), 14(9), 22(1) — 6 tamanhos sem escala (e `Icon` tem comentário "sized in em").
- Impacto: pesos de traço visivelmente diferentes na barra de estado e cabeçalho; alinhamento irregular com texto.
- Correção: mover os SVG inline para `PATHS` do `Icon`; escala fixa (12/14/16) + traço único 1.4; lint que proíba `<svg` fora de Icon/ui.
- Esforço M · Confiança alta

#### A3. Dois sistemas de componentes em paralelo (CSS legacy vs `ui/`): Badge/chip, Button/pill, globais `button {}`
- Local: `.chip` styles.css:580-582 (4 usos TSX) vs `ui/badge.tsx` (33 `<Badge>`) — mesmos valores (`fs-xs`, `r-xl`, padding 3px 6px) escritos duas vezes, `badge.tsx` com `py-[3px]` repetido 10×. `.pill` definido 2× (styles.css:643 e :873, + override :824). Regras globais de elemento `button`, `button.primary`, `button.icon`, `button.danger`, `button:not([class])` (styles.css:175,180,289,548,726) coexistem com `ui/button.tsx` (variantes `default/outline/secondary/ghost/destructive/icon/row/glyph`). `button.icon` (CSS) vs `variant="icon"` (cva).
- Impacto: um botão pode herdar da regra global ou da cva consoante `class`; hover/alturas/padding divergem (5px 10px em :726 vs `h-/px-` da cva). Deriva garantida a cada novo ecrã.
- Correção: migrar os 4 `.chip` e `.pill` para `Badge`/`Button`; apagar regras globais de `button` depois de migrar; remover duplicado `.pill`.
- Esforço M-L · Confiança alta

### Médio

#### M1. Tokens de cor duplicados/aliases com semântica confusa
- Local: styles.css:11-12,23-24,35,63. `--ok` vs `--ok2` (no claro idênticos: `#1a7f37`, e `--ok2-rgb` = `--ok-rgb`); `--danger` vs `--err` (no claro idênticos `#cf222e`; no escuro `#f87171` vs `#e5484d` — dois vermelhos diferentes); `--claude` vs `--brand-claude` (mesmo `#d97757`; no claro `#b4482a` vs `#b4532f`, **diverge**). Uso: `--danger` 21× em CSS e 0 em TSX; `--err` 5× CSS e 18× TSX; `--ok2` só em 6 CSS; `--claude` 1 uso.
- Impacto: erro aparece com dois vermelhos consoante o ecrã (CSS usa `--danger`, TSX/Tailwind `--err`); marca Claude com 2 tons no claro.
- Correção: unificar `--err`→`--danger` (ou o inverso), apagar `--ok2` e `--claude`, ajustar `*-rgb`. 
- Esforço S · Confiança alta

#### M2. Tokens usados mas indefinidos / fallback morto
- `var(--panel-2, var(--panel))` styles.css:940 (`--panel-2` não existe em lado nenhum → sempre fallback); `--accent-fg` definido só no tema claro (styles.css:102), nunca no escuro nem usado em TSX; `--accent-st` local (kcard :560) ok.
- Impacto: baixo visual, mas indica token fantasma; notice-toast usa `--line-ctl` com fallback desnecessário.
- Correção: remover fallback / `--accent-fg`. Esforço S · Confiança alta

#### M3. Escala de espaçamento `--sp-*` existe mas tem **0 usos**
- Local: styles.css:57-60 definidos; `grep var(--sp-` em todos os CSS = 0 (só 1 ocorrência, na definição). Valores de padding/margin/gap em CSS: 8px×105, 6px×90, 4px×68, 10px×59, 2px×35, 12px×32, 5px×19, 3px×12, 14px×12, 7px×6, 9px×4, 18px×4, 16px×2 — 13 valores distintos, 6 fora da escala 4/8/12/16 (5, 6, 7, 9, 10, 14, 18 px).
- Impacto: ritmo vertical irregular entre painéis; UI.md promete uma escala que não é seguida nem testada.
- Correção: adotar escala 4/8/12/16 (+2/6 explícitos) e `uiRules` a avisar; ou remover `--sp-*` e documentar. Esforço L (muitas linhas) · Confiança alta

#### M4. Mistura de escalas tipográficas CSS vs Tailwind e tamanhos Tailwind fora do tema
- Local: tailwind.css:35-38 mapeia só `text-xs/sm/base/lg` → `--fs-sm/md/base/lg`, ou seja `text-xs`=11px mas `--fs-xs`=10px (nome enganador; badge precisa de `text-[length:var(--fs-xs)]`, 9 usos). `text-xl` (button.tsx:33, ui/Gallery.tsx:102) e `text-2xl` (Welcome.tsx:47) caem no default Tailwind 20/24px, fora da escala (máx. do app é `--fs-xl`=16). Contagem: `text-xs`33, `text-sm`15, `text-base`12, `text-lg`3, `text-xl`2, `text-2xl`1. `!important` em `.m-title{font-size}` styles.css:534.
- Impacto: título Welcome (24px) e glifo (20px) fogem à hierarquia; dois vocabulários para o mesmo tamanho.
- Correção: mapear `--text-xl/2xl` ao tema (16/20) ou usar `--fs-xl`; renomear escala (`fs-2xs..`) ou documentar o offset. Esforço S · Confiança alta

#### M5. Cinco stacks de fonte mono diferentes
- Local: `"Cascadia Code", Consolas, monospace` ×7 (docker.css:14,15,52; styles.css:511,535 `!important`,559,582); `"Cascadia Mono"` styles.css:462 e `DEFAULT_FONT` termprefs.ts:8; `monospace` nu 1×. Tailwind `font-mono` (badge `id`) usa o stack default do Tailwind.
- Impacto: código/IDs com glifos/larguras ligeiramente diferentes entre terminal, diff, Docker e badges.
- Correção: `--font-mono` + `--font-mono-term`; `@theme --font-mono`. Esforço S · Confiança alta

#### M6. Cores/mix "à mão" em Tailwind arbitrário e z-index sem escala
- Local: 123 classes arbitrárias `x-[…]` em TSX; destaque: `text-[var(--err)]`×15, `text-[var(--warn)]`×6, `text-[var(--ok)]`×4, `border-[color-mix(in_srgb,var(--warn)_40%/45%,transparent)]`, `…ok_35%/40%`, `…err_45%` (badge.tsx:15,23-25, HostBadge.tsx:39), alfas 12/22/35/40/45% sem token; `rgba(var(--x-rgb), .12/.13/.16/.22/.25/.3/.35/.4/.45/.7)` em CSS (styles.css:487,490,516,605,616,628,629,687,594-596,778). z-index: 60×5, 5×2, 41×2, 6, 50, 20, **1000**, 10, 1 (notifications.css:3 e popovers em 41/50/60/1000) sem token.
- Impacto: tints de estado com intensidades diferentes para a mesma semântica; sobreposição de camadas frágil (toast vs popover vs modal).
- Correção: tokens `--ok-line`, `--warn-line`, `--err-line` (+ `*-bg` já existem) e escala `--z-*`. Esforço M · Confiança média

### Baixo

- **B1. Hover/pressed com receitas diferentes:** `--s1`(2), `--s2`(1), `--line`, `--accent`, `rgba(var(--fg-rgb),.06)` (styles.css:787), `rgba(var(--accent-rgb),.35/.7)`. Uniformizar em `--s1`/`--s2`. S · alta
- **B2. Raios fora de escala:** `border-radius: 50%`×3, `0`, composto `r-md r-md 0 0`; Tailwind `rounded-xl` ×11 mapeia a `--r-xl` (10px) para badges (pílula pequena a 10px, e `.chip` igual) vs `rounded-full`×5 nos dots. Aceitável, mas `rounded-none/-l/-r/-t` para grupos não têm contrapartida de token. S · média
- **B3. Valores fixos de layout:** `max-width: 220px` (styles.css:331), `max-w-[110px]`×2, `w-[120px]`, `w-[34px]`, `h-[18px]`×2, `max-w-[420px]/[220px]` — sem tokens; `sb-pop bottom:22px`, `wt-usage-pop top:40px right:12px` (styles.css:790,974) acoplados à altura da status/top bar sem variável. S · média
- **B4. Inline `style={}` em TSX (16)** ok para CSS vars/larguras dinâmicas, mas `padding-left`/`margin-left` calculados com constantes diferentes: `8+depth*12` (FilesPanel 78,175,180,302,325), `depth*18` (Orchestra.tsx:85), `14+depth*14`/`depth*14` (AgentList.tsx:63, WorktreeRow.tsx:70) — 3 indentações de árvore diferentes (12/14/18 px por nível). Definir `--indent`. S · alta
- **B5. Gradiente inútil:** `.kcard{background: linear-gradient(180deg,var(--s3),var(--s3))}` styles.css:560 = cor plana. S · alta
- **B6. `!important` ×4** (styles.css:534,535,690,931; só 931 e 690 justificáveis). S · alta
- **B7. Hex em `termprefs.ts`** (24, temas do terminal xterm) — legítimo mas não segue o tema da app nem tem teste; documentar. S · média

### Top 5 da área
1. **A3** unificar `.chip/.pill/button global` com `ui/Badge/Button` (fonte da deriva).
2. **A2** ícones: tudo via `Icon`, 1 traço, escala de tamanhos.
3. **A1** tokens `--shadow-*` com variante claro.
4. **M1** apagar aliases `--ok2/--claude`, unificar `--err`/`--danger`.
5. **M3** decidir sobre `--sp-*` (0 usos) e reduzir 13 espaçamentos para escala curta.

### Ideias de melhoria
- Alargar `uiRules.test.ts`: proibir `box-shadow` com `rgba(0` literal, `<svg` fora de `Icon`/`ui`, `z-index` numérico, `text-[length:`/`[…px]` Tailwind, `font-family` literal.
- Gallery (`ui/Gallery.tsx`) como baseline visual: screenshot por tema em CI (E2E) para apanhar regressões claro/escuro.
- Documentar em `UI.md` os tokens de motion (só 2 `transition` em CSS: `transform .1s`, `.12s`; Tailwind usa `duration-100`) — criar `--ease/--dur`.
- Gerar `--*-rgb` a partir dos hex (um só lugar) para evitar dessincronia hex↔rgb entre tema escuro e claro.
- Nota: não foi possível verificar contraste/rendering reais (app não corrida); uma passagem visual com `tools/e2e.mjs` em claro+escuro deve confirmar A1/M1.

---

## Área 3 — Fluxos principais (auditoria UI/UX, só leitura)

**Método e limites.** A app NÃO foi executada: não existe `app/dist` nem build do frontend neste worktree, e o `axelstra-app.exe` do checkout principal é anterior/não tem o frontend embebido para esta branch. Auditoria só por código (`app/src`), por isso **confiança média** na maior parte (comportamento visual/CSS não verificado). Sem screenshots. Nenhum ficheiro do repo foi tocado. Todos os caminhos são relativos a `app/src/`.

Pontos fortes confirmados no código (não repetir como problema): Confirm foca o botão seguro em ações danger/warning (`Confirm.tsx:81,130`); remover projeto tem pré-visualização, modo "só lista" vs "tudo", type-to-confirm só quando há risco real e backdrop não fecha (`RemoveProject.tsx:208`, `removeProjectLogic`); apagar cartão Kanban tem Desfazer (`Kanban.tsx:141-150`); toasts pausam em hover/foco (WCAG 2.2.1, `ErrorToast.tsx:125-128`); Add project tem validação inline com `role="alert"` (`addProject/CloneTab.tsx:67,92,119`, `NewTab.tsx:101`); descartar texto pergunta (`CreateWorktree`, Automations, Kanban).

---

### Crítico
Nenhum.

### Alto

#### A1. "Integrar" corre sem confirmação, sem pré-visualização e sem desfazer
- **Local:** `sidebar/PendingIntegration.tsx:39-57` (handler), botão `:82-87`; ecrã: sidebar, bloco "Por integrar" de um worktree.
- **Evidência:** um clique chama `integrateWorktree` diretamente; o resultado `doneRemoved` implica que o worktree **e o branch são removidos** (`:46-47`). Contraste: Merge de PR (`ShipPanel.tsx:149`) e rebase (`sidebar/actions.ts:146`) perguntam primeiro; "Limpar" (`actions.ts:66`) pergunta com lista.
- **Impacto:** ação que move trabalho para o branch por defeito (e pode empurrar com `push_after_integrate`) e apaga a pasta, num botão pequeno ("chip") ao lado de "Descartar", sem dizer quantos commits/para onde antes de agir. Erro de clique = sem volta fácil.
- **Correção:** `ask()` kind "warning" com `T.tip(ahead, into)` (já existe, `:30`) e, se `dirty`, aviso; após sucesso, toast com ação "Ver commit"/nota do SHA. Opcional: preferência "não perguntar de novo" se auto-integrate estiver ativo.
- **Esforço:** S · **Confiança:** alta

#### A2. Orquestração (Kanban) não se pode parar, e o paralelismo é fixo em 3
- **Local:** `Kanban.tsx:56` (`orchStart(props.project, 3, ...)`), barra `:411-421`; `api.ts:705-715` (só existem `orch.start/retry/status/limits/trust_info`; não há cancel/stop em `crates` nem na UI).
- **Evidência:** durante `run().state === "running"` o único botão fica desativado com "A executar…" (`:416-418`). O número de workers (3) está hardcoded; o limite real está em Orquestra (`Orchestra.tsx:313-322`), em outro ecrã.
- **Impacto:** uma run lançada por engano (ou que está a gastar tokens/CPU) só se interrompe matando sessões uma a uma em "Orquestra", sem ligação visível a partir do Kanban; o utilizador não sabe disso. Falha de reversibilidade num fluxo caro.
- **Correção:** botão "Parar execução" na barra (novo método daemon `orch.cancel`, ou no mínimo atalho "Ver na Orquestra" no banner `krun`); campo "paralelo: N" junto de "Executar".
- **Esforço:** L (daemon) / S (atalho para Orquestra) · **Confiança:** alta (ausência de método verificada por grep)

### Médio

#### M1. Type-to-confirm não foca o campo de texto
- **Local:** `Confirm.tsx:117-124` (Input sem foco) e `:130` (foco vai para Cancelar porque `kind:"danger"`); idem `RemoveProject.tsx:240-247`.
- **Impacto:** no diálogo "Remover worktree" com alterações por guardar, o foco cai em Cancelar, que no DOM vem *depois* do input: o utilizador tem de Shift+Tab ou usar o rato. Fricção no fluxo mais perigoso, e Enter em Cancelar cancela sem explicação.
- **Correção:** quando há `typeToConfirm`, focar o Input (Cancelar continua a ser o default do Enter apenas se o input estiver vazio — já o é, pois o botão OK está disabled). 
- **Esforço:** S · **Confiança:** alta

#### M2. Remover worktree fecha as sessões antes de saber se a remoção funciona
- **Local:** `sidebar/actions.ts:120-131` (`closeSessions` e só depois `removeWorktree`).
- **Impacto:** se `removeWorktree` falhar (pasta presa por antivírus/processo), os agentes e terminais já foram mortos e o worktree continua lá; o utilizador vê só um toast de erro sticky. Perda parcial irreversível.
- **Correção:** ordem inversa não é possível (pasta presa pelos processos), por isso: avisar na pergunta "isto fecha N sessões" (já existe via `removeLost`) e, em caso de falha, o toast deve dizer "as sessões foram fechadas; o worktree mantém-se" + ação "Tentar de novo".
- **Esforço:** S · **Confiança:** média

#### M3. Remover projeto "tudo" sem feedback de progresso
- **Local:** `sidebar/actions.ts:85-102` (compare com `dropWorktree` que faz `showNotice(T.removing)` em `:119`).
- **Impacto:** `project.remove` mode `all` fecha sessões, remove worktrees (pode demorar) e só no fim mostra o resumo (`:101`). Entre o clique em "Remover" e o fim não há spinner nem toast; parece bloqueado e convida a cliques repetidos.
- **Correção:** `showNotice(removing)` ou estado "a remover…" na linha do projeto; manter o diálogo aberto com botão em `busy`.
- **Esforço:** S · **Confiança:** média

#### M4. "Novo worktree" não valida o nome antes de submeter
- **Local:** `sidebar/CreateWorktree.tsx:25-38,52`; `Sidebar.tsx:186-206`.
- **Evidência:** só verifica não-vazio. O placeholder diz que o nome é também o branch (`strings.ui.ts:58`), mas caracteres inválidos/branch existente só falham depois, no daemon: o diálogo fecha, aparece a linha "A criar…", falha, reabre com o nome (`Sidebar.tsx:196-199`) e um toast de erro. Add project, ao contrário, valida inline.
- **Impacto:** ida-e-volta lenta (checkout grande) para um erro previsível; diálogo pisca fecha/abre.
- **Correção:** validar localmente (espaços, `..`, `~^:?*[\`, nome já existente na lista de worktrees) com mensagem `role="alert"`; mostrar "a partir de: <branch por defeito>" no diálogo.
- **Esforço:** S · **Confiança:** alta

#### M5. Restaurar backup sem confirmação; "Limpar agora" nas Definições sem confirmação
- **Local:** `Settings.tsx:149-157` + botão `:215` (restore); `:163-178` + botão `:324` (cleanNow).
- **Impacto:** restore substitui definições, projetos e sessões do daemon (`T.backupNote`, `strings.logic.ts:49`); só a escolha da pasta no diálogo nativo faz de "confirmação". `cleanNow` fecha sessões/processos e remove worktrees de todos os projetos (`closed_sessions/closed_procs` em `:166-171`), enquanto a mesma operação na sidebar (`actions.ts:66`) pede confirmação `danger` com lista. Inconsistência de proteção.
- **Correção:** `ask()` warning para restore ("substitui o estado atual; faz primeiro uma cópia?"); para cleanNow reutilizar a pré-lista de `cleanFinished`.
- **Esforço:** S · **Confiança:** média

#### M6. Remover worktree: "type-to-confirm" depende de um estado que pode ainda não existir
- **Local:** `removeRisk.ts:15` (`!!st && ...`), usado em `sidebar/actions.ts:109-111`; `wtStatus` vem de `store/git.ts:78-89`.
- **Impacto:** se o estado do worktree ainda não foi lido (arranque, worktree acabado de criar, falha do poll), `st` é `undefined` → sem aviso de alterações e sem type-to-confirm, mesmo com trabalho por guardar. Falha silenciosa para o lado perigoso.
- **Correção:** com `st` indefinido, ler o estado on-demand antes de perguntar (ou tratar como "estado desconhecido" e exigir type-to-confirm).
- **Esforço:** S · **Confiança:** média (não verifiquei com que frequência `wtStatus` fica vazio)

#### M7. Toasts de erro "sticky" podem ser expulsos em silêncio
- **Local:** `toasts.ts:82-86` (`MAX_TOASTS = 5`, `slice(-5)`).
- **Impacto:** sexto toast expulsa o mais antigo, mesmo um erro por ler. Mitigado porque erros vão para o centro de notificações (`:89-90`), mas avisos (`kind:"warn"`, sticky) **não** são registados (só `kind === "error"`), logo um aviso de integração/rebase com conflito (`PendingIntegration.tsx:54`, `actions.ts:160`) pode desaparecer sem rasto.
- **Correção:** registar também `warn` sticky no centro de notificações; ou expulsar primeiro os `info`.
- **Esforço:** S · **Confiança:** alta

#### M8. Orquestra: limite de workers sem feedback e erros engolidos
- **Local:** `Orchestra.tsx:262-268` (`applyMax`), `:320-321` (Enter **e** blur), `:255-261` (`stop`).
- **Evidência:** `workerLimits(n).catch(() => 0)` — falha silenciosa; sem toast de sucesso; `max="64"` no input mas `applyMax` só rejeita `n < 1`; Enter dispara a gravação e o blur seguinte dispara outra. `stop` também engole falhas de `killSession/removeSession` e remove a linha na mesma.
- **Impacto:** o utilizador altera o limite e não sabe se pegou; um "Parar" que falhou parece ter funcionado.
- **Correção:** clamp 1–64, um só disparo (ignorar blur se já aplicado), `showNotice` no sucesso e `setError` no erro.
- **Esforço:** S · **Confiança:** alta

#### M9. Kanban mostra erros num banner próprio com `String(e)` em vez do toast
- **Local:** `Kanban.tsx:58-59,72,421-423` (`setErr(String(e))`, `.kerr`); `UI.md` regra "Feedback: erros vão para o toast, nunca texto dentro de um painel".
- **Impacto:** erro de arranque da orquestração fica num banner que só limpa num reload com sucesso; não é sticky-com-fechar, não vai ao centro de notificações, não é anunciado como `role="alert"` (o `.kerr` não tem papel). Inconsistente com o resto.
- **Correção:** `setError(errorText(e))` + manter o banner só para estados persistentes (cartões `broken`).
- **Esforço:** S · **Confiança:** alta

#### M10. Ações mais frequentes não têm atalho de teclado
- **Local:** `commands.ts:46-53,57-58` — `newAgent`, `newTerminal`, `split`, `newWorktree`, `removeWorktree`, `kanban`, `orchestra` sem `keys`; só aparecem na paleta (Ctrl+K) e não no cheat sheet (`?`, que lista só comandos com `keys`, `commands.ts:92`).
- **Impacto:** criar terminal/agente/worktree — o dia-a-dia — exige Ctrl+K + escrever ou rato. A cheat sheet também não diz que existe a paleta para o resto.
- **Correção:** atribuir `Ctrl+Shift+N` (worktree), `Ctrl+Shift+A` (agente), `Ctrl+Shift+\`` (terminal) ou equivalentes sem colisão com terminal; acrescentar à cheat sheet uma linha "Tudo o resto: Ctrl+K".
- **Esforço:** M · **Confiança:** alta

### Baixo

- **B1. Contagem "ahead" sem unidade.** `sidebar/PendingIntegration.tsx:77-79` mostra `{x().ahead}` solto (ex.: "3 · alterações"). Ler como "3 commits por integrar". S, alta.
- **B2. Add project sem guarda de alterações.** `AddProject.tsx:318` não passa `dirty`; clicar fora do diálogo com URL/nome de clone preenchidos perde o formulário (`ui/dialog.tsx:85`). S, média.
- **B3. "Limpar tudo" das notificações sem confirmação nem desfazer.** `NotificationBell.tsx:146-154`; apaga também o registo de erros (único rasto, ver M7). S, média.
- **B4. Dispatcher mostra só os últimos 3 pedidos e não diz que há mais.** `Dispatcher.tsx:254` (`slice(-3)`), sem "ver todos". S, alta.
- **B5. Botão de sair chama-se "Fechar".** `CloseConfirm.tsx:321` usa `S.close`; no diálogo "Fechar Axelstra?" é ambíguo com fechar o diálogo (o outro é "Cancelar"). Usar "Sair". S, média.
- **B6. Falha de merge de PR só numa nota pequena do painel.** `ShipPanel.tsx:156,252-254` (`pnote`) + notificação; contra a regra de UI.md. Erro de merge devia ser toast sticky. S, média.
- **B7. Palette: hint "worktree" hardcoded.** `Palette.tsx:378` literal fora do dicionário (o resto usa `S.`). S, alta (cosmético/i18n).
- **B8. Ctrl+K abre a paleta por cima de um diálogo de confirmação aberto.** `Palette.tsx:358-366` não verifica `overlayOpen()`; o `Confirm` fica por baixo (`inert`), o que pode confundir quem estava a meio de uma ação perigosa. S, baixa (não verifiquei o comportamento visual).

---

### Top 5 da área
1. **A1** Integrar sem confirmação nem resumo (apaga worktree/branch).
2. **A2** Orquestração Kanban sem Parar e com paralelismo fixo a 3.
3. **M1** Type-to-confirm não foca o input (fluxo mais perigoso).
4. **M6** Type-to-confirm de remoção depende de `wtStatus` possivelmente vazio.
5. **M2/M3** Remoção fecha sessões antes de saber se funciona; remover projeto sem progresso.

### Ideias de melhoria
- Um único componente de "confirmação com resumo" (o que se perde / o que sobra / para onde vai) reutilizado por Integrar, Rebase, Merge, Limpar, Restaurar.
- Toast com Desfazer onde é viável (Kanban já faz): remover do *registo* de worktree "manual" não, mas "Descartar" de worktree vazio sim.
- Paleta: acrescentar comandos de contexto (Integrar worktree atual, Remover projeto, Abrir secção X das Definições, Executar Kanban) e mostrar o atalho à direita (já feito para os que têm `keys`).
- Mostrar no banner da orquestração (`krun`) ligações para a Orquestra e para o worktree de integração; progresso "2/5 cartões".
- Diálogo Novo worktree: pré-visualizar branch resultante e base ("a partir de main @ abc1234"), com validação inline como no Add project.
- Centro de notificações: filtros (erros/CI/feitos), "marcar como lido" por item, e não limpar sem desfazer (toast "Limpo — Desfazer").
- Teste E2E dos fluxos de remoção/integração com o `fake-agent` (`tools/e2e.mjs`) para validar M1/M2 em app real; ainda por correr.

---

## Área 4 — Estados vazios/carregamento/erro, mensagens e i18n

**Método:** auditoria SÓ por código (sem a app a correr). Há um binário em `../../axelstra/target/debug/axelstra-app.exe` (checkout principal), mas pode estar desatualizado face a este branch e um build aqui implicaria `axelstra lock build`. Não corri a app, não há screenshots. Confiança geral: **média** (sem evidência visual); onde a evidência é só texto/grep a confiança é alta. Ficheiros do repo não alterados.

Pontos verificados e **sem problema** (para não repetir trabalho): camada pt-BR completa e bem escrita (grep de marcadores pt-PT em `strings*.ptbr.ts` só deu falsos positivos); erros vindos do Rust usam todos `coded!` (só restam `bail!` em `job.rs`/`runner.rs`, não chegam ao utilizador); atalhos em tooltips passam por `K()` (Mac); `const needs = [W.needGit…]` em `Welcome.tsx:10` é seguro (guarda proxies vivos); plurais usam `count/plural` (sem `toFixed`/`toLocaleString`); `Loading` tem estado "demora" + retry.

---

### Crítico
Nenhum.

### Alto

#### A1. Sem estado "a carregar projetos": utilizador com projetos vê o ecrã de primeira execução; com o daemon offline a app parece uma instalação nova
- **Local:** `app/src/store/git.ts:24` (`projects` começa `[]`, sem flag "carregado"), `app/src/App.tsx:394` (`<Show when={projects()[0]} fallback={<Welcome …/>}>`), `app/src/store/git.ts:118-119` (falha → só `setError`).
- **Evidência:** código. Não há `projectsLoaded` em lado nenhum (grep).
- **Impacto:** em cada arranque há um instante em que quem já tem projetos vê "Bem-vindo ao Axelstra / Adicionar projeto / Para começar: git…". Se o daemon estiver em baixo (cenário "daemon offline") isso persiste e a área principal mostra boas-vindas de primeira execução, em vez de "sem ligação ao daemon" com ação de reiniciar; o único sinal é um toast. Convida a adicionar de novo um projeto que já existe.
- **Correção:** sinal `projectsLoaded` (true após 1.º `listProjects` ok/erro); enquanto false mostrar `Loading`; se falhou e lista vazia, mostrar estado de erro dedicado ("Sem ligação ao daemon" + `Tentar agora` / `Reiniciar daemon`, já existem em `S.toast.retryNow`, `S.restartDaemon`) em vez do Welcome.
- **Esforço:** S–M · **Confiança:** alta (leitura direta; o flash exato depende de timing).

#### A2. Seletor de agente sem estado "nenhum agente instalado"
- **Local:** `app/src/AgentPickerModal.tsx:26-34`, `app/src/store/ui.ts:15-22` (`agentKeys` = só instalados; `[]` até o daemon responder), `app/src/useBootstrap.ts:89-90`.
- **Evidência:** código.
- **Impacto:** sem nenhum CLI instalado (o caso típico de primeira execução) o "Novo agente" abre um `Select` vazio e o botão **Abrir** continua ativo (`go()` usa `choice()` = agente por defeito não instalado) → terminal com erro cru "not recognized". O aviso só existe como toast único no arranque (`useBootstrap.ts:93-94`) e desaparece.
- **Correção:** se `agentKeys().length === 0`: mostrar no modal "Nenhum agente encontrado. Instala o Claude Code, Codex ou Gemini CLI" com link/instruções, desativar **Abrir**. Ligar também o estado "ainda a carregar" (lista vazia por demora ≠ lista vazia real).
- **Esforço:** S · **Confiança:** alta.

### Médio

#### M1. Aviso de arranque "nem o claude nem o codex" ignora o Gemini e dá instrução errada
- **Local:** `app/src/useBootstrap.ts:93`, `app/src/strings.ts` (`app.noAgents`: "nem o claude nem o codex foram encontrados (instala um deles…)"). O host devolve também `gemini` em `missing_tools` (`app/src-tauri/src/main.rs:1064`).
- **Impacto:** quem só tem Gemini instalado recebe o aviso de que falta configurar agentes; quem não tem nenhum não é informado de que o Gemini também serve.
- **Correção:** condição `claude && codex && gemini` ausentes; texto "nenhum agente (claude, codex, gemini) encontrado".
- **Esforço:** S · **Confiança:** alta.

#### M2. Onboarding de primeira execução é uma linha; os 3 passos explicativos estão mortos
- **Local:** `app/src/Welcome.tsx:36-40` (só `W.flow`: "Adiciona um projeto, cria um worktree e abre um agente."); strings órfãs `app/src/strings.panels.ts:25-34` (`sidebar.firstRun/welcome/step1-3/needsPre/needsMid/needsPost/needsEnd/addFirst`, mais as cópias `.en`/`.ptbr`) — nenhuma referência em `.tsx`/`.ts` (grep).
- **Impacto:** nunca se explica o que é um worktree nem porque se abre um agente "num worktree" (a frase "cada tarefa tem o seu branch e a sua pasta" existia e perdeu-se). Ao mesmo tempo mantêm-se 9 chaves × 3 línguas a ser traduzidas sem uso.
- **Correção:** reaproveitar os 3 passos no `Welcome` (lista numerada com o passo atual destacado) e apagar as chaves restantes.
- **Esforço:** S · **Confiança:** alta.

#### M3. "main" significa três coisas diferentes (jargão sem explicação)
- **Local:** `strings.welcome.ts` `mainTag: "principal"` (o worktree principal, mas em EN é "main"); `strings.panels.ts:72-75,85,92,561-571` (`leadTag: "main"`, "Iniciar main do worktree", "o agente main", "Não foi possível iniciar o main de {n}"); `strings.ui.ts` `dispatcher.note/empty` ("Cada pedido feito na main… Pede algo ao agente da main."); `strings.panels.ts:36-37,799` ("Despachante" vs "dispatcher" na mesma frase do `autoIntegrateHint`, e "líder" em `leadTip`).
- **Impacto:** o utilizador novo não distingue branch `main`, worktree principal, agente "main" (líder) e "agente da main" do projeto. "Despachante/dispatcher/líder/main" são 4 nomes para papéis próximos; o glossário em `strings.ts:5-9` não os fixa.
- **Correção:** fixar no glossário (ex.: "agente principal" do projeto, "líder" do worktree, "worker/auxiliar"), usar o mesmo termo em tag, tooltip, botão e erros; nunca "main" nu.
- **Esforço:** M · **Confiança:** média (decisão de produto).

#### M4. Botão "Copiar diagnóstico" do ecrã de erro não dá feedback, e falha em silêncio
- **Local:** `app/src/Boundary.tsx:29-33` (`void copyDiagnostics().catch((e) => console.debug(…))`).
- **Impacto:** no ecrã "O Axelstra encontrou um erro" é a ação principal para reportar; o clique não mostra "Copiado" nem erro (e esse ecrã pode ser o único ponto de contacto quando a app caiu). Existe `P.confirm.copied` já traduzido.
- **Correção:** estado local `copied/failed` no botão (texto muda 2 s) ou `toast`.
- **Esforço:** S · **Confiança:** alta.

#### M5. Mensagens de erro de git/gh/docker são cortadas a 160 caracteres sem forma de ver o resto
- **Local:** `app/src/friendly.ts:30` (`clip`), usado em `friendly.ts:41-42`.
- **Impacto:** o `(detalhe)` entre parênteses acaba em "…" — justamente a parte útil de um conflito de merge ou de um push rejeitado (lista de ficheiros, ref). O toast tem scroll (`styles.css:941`) mas o texto já veio cortado.
- **Correção:** guardar o texto completo no toast/notificação (botão "Copiar detalhe") ou cortar a 600+; manter o clip só no 1.º nível.
- **Esforço:** S · **Confiança:** média.

#### M6. Glossário violado: "push/merge/pull request" misturados com enviar/integrar/PR
- **Local:** `strings.panels.ts:800-801` ("Fazer push depois de integrar"), `msg/integrate.ts:19,57` ("o push falhou… faz o push à mão"), `strings.panels.ts:596-624` ("Fazer merge", "Auto-merge", "Merge feito", `failedNote`), `msg/integrate.ts:10,48`, `msg/vcs.ts:27`, `msg/agents.ts:79,220` ("pull request" por extenso).
- **Impacto:** o mesmo botão/ação chama-se "Enviar" (`panels.ts:492`) e "push" noutro ecrã; "integrar" no worktree e "merge" no PR. `strings.ts:5-9` diz explicitamente "PR (not 'pull request')", "enviar (push)", "integrar = merge".
- **Correção:** alinhar ao glossário (ou atualizar o glossário se "merge de PR" for deliberado, distinguindo-o de "integrar") e acrescentar um teste de lint a proibir `\bpush\b` / `pull request` nas chaves pt.
- **Esforço:** S · **Confiança:** alta.

#### M7. Género de "branch" inconsistente
- **Local:** feminino em `strings.panels.ts:799`, `msg/integrate.ts:10,20,44,48,58`, `msg/kanban.ts:58,170`, `msg/vcs.ts:111` (12 ocorrências "a/uma/na branch"); masculino em `strings.panels.ts:122,131,138`, `strings.logic.ts:136`, `msg/vcs.ts:27` ("Este branch…") (≈24 ocorrências).
- **Impacto:** lê-se como descuido, sobretudo na mesma tela (ex.: `integrate.ts` "uma branch do Axelstra" vs `panels.ts` "o branch principal"). O glossário trata "worktree" mas é omisso em "branch".
- **Correção:** escolher um (o masculino já domina) e acrescentar a regra ao glossário; substituição mecânica nas 12 ocorrências (e pt-BR, onde "o branch" também é o comum).
- **Esforço:** S · **Confiança:** alta.

#### M8. Dicas de atalho com "Ctrl" fixo que não passam por `K()` (Mac mostra Ctrl)
- **Local:** `strings.ts:66` (`terminalNote`, usado em `Settings.tsx:287` sem `K`), `strings.panels.ts:796` (`setupNote`, "Ctrl+Enter guarda"), `strings.panels.ts:837` (`htmlTip`), `App.tsx:470` usa `K` mas com teclas literais em vez do registo `commandById(...).keys` (que `Welcome.tsx:38` já usa).
- **Impacto:** em macOS o texto diz Ctrl onde a tecla é Cmd; se um atalho mudar no registo, as dicas fixas ficam erradas.
- **Correção:** passar por `K()` e, onde existe comando, derivar do registo.
- **Esforço:** S · **Confiança:** média (não confirmado em macOS).

### Baixo

#### B1. Welcome só lista os worktrees do primeiro projeto
- **Local:** `App.tsx:394-402` (`projects()[0]`, `worktrees={p().worktrees}`).
- **Impacto:** com vários projetos e sem worktree ativo, "Escolhe um worktree de <projeto 1>" ignora os outros; o título não diz que é só um.
- **Correção:** usar o projeto da última seleção, ou listar por projeto. **S/M · média.**

#### B2. Texto fixo "+ Terminal" fora do dicionário
- **Local:** `App.tsx:467` (existe `S.app.newTerminal`). Também o "+" em `S.sidebar.addProject` ("+ Adicionar projeto", `strings.panels.ts:21`) duplica o ícone.
- **Correção:** `S.app.newTerminal` + `<Icon name="plus">`. **S · alta.**

#### B3. Voz: mensagens só falam do Windows
- **Local:** `strings.logic.ts:218` ("definições de privacidade do Windows"), `strings.panels.ts:744` ("microfone selecionado no Windows"). A app tem alvos macOS/Linux (`ci.yml`).
- **Correção:** texto neutro ou escolhido por plataforma. **S · alta.**

#### B4. Erro de descarga do modelo de voz culpa sempre a ligação
- **Local:** `app/src/voiceErrors.ts:20` (`downloadErrorText` ignora o erro; o host também devolve "…does not match the expected hash", `app/src-tauri/src/voice.rs:112`, e erros de escrita em disco).
- **Impacto:** "Verifica a ligação" quando o problema é disco cheio ou ficheiro corrompido. **S · média.**
- **Correção:** distinguir hash/disco/rede (códigos `coded!`).

#### B5. Tom: 3.ª pessoa isolada num produto todo em "tu"
- **Local:** `strings.panels.ts:864` ("Ainda não lançou nenhuma tarefa."); resto usa "Pede…/Abre…". Também `strings.logic.ts:142` ("Vai tentar de novo" — quem? o app), `:209` ("worktree vazio X removido" em minúscula).
- **Correção:** "Ainda não lançaste…", "O Axelstra vai tentar de novo…". **S · alta.**

#### B6. "Anfitrião" (pt-PT) vs "host" em todo o resto da UI/CLI
- **Local:** `strings.remote.ts:20-40`, `strings.logic.ts:102`; no pt-BR já é "host" (`strings.remote.ptbr.ts:18-32`). A CLI e `host.info` usam "host".
- **Impacto:** termo que o utilizador não encontra na documentação nem na CLI (`axelstra … --host`). **S · média** (decisão de glossário).

#### B7. Estados vazios sem ação
- **Local (exemplos):** `strings.panels.ts:510` ("Sem alterações"), `:526` ("Sem eventos neste worktree."), `:555` ("Sem comentários de review."), `:683` ("Nenhum agente a correr… Abre um agente primeiro." sem botão), `App.tsx:461` ("Sem terminais neste worktree." — o inverso do que se quer dizer; os botões abaixo compensam), `strings.remote.ts:34` ("Ainda não há anfitriões." sem botão ao lado).
- **Bons exemplos a copiar:** `strings.panels.ts:190` (Docker diz como criar o primeiro), `:678` (agentes auxiliares explicam o comando).
- **Correção:** cada vazio com uma frase "porquê" + 1 CTA (ex.: remote → "Adicionar anfitrião"; `noAgent` → botão "Abrir agente"). **M · média.**

#### B8. Texto composto por pedaços (Pre/Mid/Post) dificulta tradução
- **Local:** `strings.panels.ts:678-679` (`emptyPre/emptyPost`), `:810-811` (`noFilePre/Post`), antes `needsPre/Mid/Post/End`.
- **Impacto:** a ordem das palavras fica presa à de pt-PT; hoje funciona em EN/pt-BR, mas é frágil. **M · baixa.**
- **Correção:** função `(code: JSX) => …` ou marcador `{cmd}` com split.

---

### Top 5 da área
1. **A1** — sem estado "a carregar/offline" para projetos: primeira execução falsa (`App.tsx:394`).
2. **A2** — seletor de agente vazio e com **Abrir** ativo quando não há agentes (`AgentPickerModal.tsx:26`).
3. **M2 + M3** — onboarding de uma linha, com os 3 passos mortos; "main" ambíguo (3 significados).
4. **M1** — aviso de arranque ignora o Gemini (`useBootstrap.ts:93`).
5. **M4 + M5** — ecrã de erro sem feedback ao copiar diagnóstico; erros cortados a 160 chars.

(Consistência de glossário M6/M7 vem logo a seguir e é mecânica: barata e muito visível.)

### Ideias de melhoria
- **Teste de glossário** em `i18nLint.test.ts`: regex proibidas nas chaves pt-PT (`\bpush\b`, `pull request`, `\bmerge\b` fora de nomes próprios, "a branch", "ficheiro(s)") e na camada pt-BR as de pt-PT (`ecrã`, `ficheiro`, `a correr`).
- **Painel "Primeira execução" com estado real**: checklist viva (git ✔, agente ✔/✘, gh opcional, 1.º projeto, 1.º worktree, 1.º agente) a reaproveitar `envTools` já usado no Welcome; desaparece quando tudo está feito.
- **Componente `EmptyState`** (ícone, título, causa, 1 CTA, atalho) para uniformizar B7 e substituir `placeholder`/`empty-actions` ad hoc.
- **Estado de ligação ao daemon visível** (ponto na status bar: ligado / a reconectar / offline) em vez de depender só do toast; o banner `ErrorToast.tsx:60` já existe.
- **Copiar detalhe** em todo o toast de erro (mantém texto amigável + original completo).
- Mensagens de plural/género: acrescentar ao glossário regras para "branch", "main", "host/anfitrião" e "merge de PR vs integrar".

---

## Área 5 — Barra lateral e terminais (auditoria UI/UX, só leitura)

**Método e limites.** A app NÃO foi corrida: este worktree não tem `target/debug` nem `app/dist`, e o `tools/e2e.mjs` precisa de `cargo build --features tauri/custom-protocol` + `npm run build` (builds pesados partilhados). Existe um `axelstra-app.exe` no checkout principal, mas não corresponde a este frontend, por isso não o usei. Sem screenshots. Auditoria **só por código**; confiança no máximo média (comportamento visual real por confirmar). Nenhum ficheiro do repo foi alterado.

Ficheiros lidos: `Sidebar.tsx`, `sidebar/{WorktreeRow,AgentList,ProjectHead}.tsx`, `StateDot.tsx`, `agentState.ts`, `agentRow.ts`, `TerminalView.tsx`, `terminal/{createXterm,useClipboard,useFind,useWebgl}.ts`, `SplitPane.tsx`, `TabBar.tsx`, `Tab.tsx`, `store/layout.ts`, `layout.ts`, `App.tsx`, `styles.css`, `termui.css`.

Pontos já bem resolvidos (não são achados): estados com forma + cor (`StateDot.tsx`, `DOT_GLYPH`), `prefers-reduced-motion` mantém as formas, tooltips/aria nos dots, separadores redimensionáveis com teclado (`layout.ts:resizeTo`), truncagem de etiquetas de tabs (`ui/tabs.tsx:30`), WebGL só no terminal visível, fit com visibilidade (`TerminalView.tsx:41-50`), recolha por projeto/grupo/agentes.

---

### Alto

#### A1. Rodapé da sidebar (Definições, Dispatcher, Automatizações) e "Adicionar projeto" rolam com a lista
- **Local:** `Sidebar.tsx:272-282` e `361-370`; `styles.css:147-156` (`.sidebar { overflow: auto }`). Ecrã: barra lateral com muitos projetos/worktrees/agentes.
- **Evidência:** código (sem screenshot). O `<div class="grow"/>` só empurra o rodapé para baixo quando o conteúdo é curto; com conteúdo longo todo o `<aside>` faz scroll, incluindo marca, botão de adicionar e rodapé.
- **Impacto:** com ~5+ projetos abertos (e em 800x500, altura mínima) o utilizador tem de rolar até ao fim para chegar a Definições ou Dispatcher, e o botão "Adicionar projeto" desaparece ao rolar. Perde-se também a noção de "onde estou".
- **Correção:** fazer da lista de projetos o único elemento com scroll (`flex:1; min-height:0; overflow:auto`) e deixar cabeçalho (marca + adicionar) e rodapé fixos (`flex: none`).
- **Esforço:** S. **Confiança:** alta (estrutura CSS clara).

#### A2. A pega de redimensionar da sidebar fica no sítio errado em janelas estreitas
- **Local:** `styles.css:718` (`.resizer { left: var(--left-w) }`) vs `styles.css:900,905` (`grid-template-columns: min(var(--left-w), 34vw) …`); `layout.ts:11-14` (`fitPanel` não conhece o `34vw`); `store/layout.ts:47`.
- **Evidência:** código. Com `winW ≤ 1000`, o painel direito esconde-se mas `leftW` pode ir até `win - 360` (ex.: 1000 → 480; 800 → 440). A coluna real é `min(leftW, 34vw)` = 340px (a 1000) ou 272px (a 800), mas o `.resizer` e `aria-valuenow` continuam em `leftW`.
- **Impacto:** quem guardou uma sidebar larga (≥ 340px) e reduz a janela (ou usa 800x600) vê um separador "fantasma" de 6px a cobrir o terminal 140–170px à direita da sidebar (z-index 20): cliques/seleção de texto no terminal falham nessa faixa, e o arrastar não corresponde ao que se vê. Tem de arrastar para trás até convergir.
- **Correção:** aplicar o mesmo limite no JS (`leftW` = `min(leftW, 0.34*winW)` quando `winW ≤ 1000`) e usar essa variável no `.resizer` e no grid, em vez de duas fórmulas.
- **Esforço:** S. **Confiança:** média-alta (lógica verificada; efeito visual por confirmar).

#### A3. Tabs com scroll horizontal escondido: o tab ativo pode ficar fora de vista
- **Local:** `TabBar.tsx:41` (`overflow-x-auto [scrollbar-width:none]`), `Tab.tsx` (sem scroll para o ativo). `grep scrollIntoView` não encontra nada em `TabBar`/`Tab`/`App.tsx`.
- **Evidência:** código.
- **Impacto:** com Kanban + Docker + vários terminais/agentes (cenário normal com líder e workers) a faixa excede a largura (a 800px sobram ~400px para tabs, cada uma até 220px = 2 tabs). Abrir um agente pela sidebar, ou criar um novo, pode deixar a tab ativa invisível; não há barra, setas, sombra de desvanecimento nem menu "todas as tabs". O utilizador não sabe que existem mais tabs.
- **Correção:** `scrollIntoView({inline:"nearest"})` no tab ativo (efeito em `selected`), indicador de overflow (gradiente nas margens ou botão "▾ lista de tabs"), e roda do rato vertical → scroll horizontal.
- **Esforço:** M. **Confiança:** alta no código; média no impacto (sem correr).

### Médio

#### M1. Linha de worktree demasiado carregada em sidebar estreita; o nome encolhe para nada
- **Local:** `sidebar/WorktreeRow.tsx:109-243`; `styles.css:202-207` (`.item .name { flex:1; overflow:hidden }`), `:860` (`.wt-topic`), `:227`/`.x { opacity:.5 }`, `store/layout.ts` + `layout.ts:LEFT.min=180`.
- **Evidência:** código. Numa só linha: chevron, dot, nome, tópico do dispatcher, "arquivado", aviso off-main, dirty, overlap, ahead/behind, PR+ícone, contador, pin, e ainda DOIS botões (`more`, `x`) sempre visíveis a 50%. Com sidebar a 180px (largura útil ≈ 156px) os botões ocupam ~40px e os badges o resto.
- **Impacto:** o nome do branch, que é a identidade da linha, fica reduzido a "…" ou a 1–2 letras; o `title` da linha é o caminho completo, não o nome do branch. Distinguir worktrees fica difícil exatamente quando há muitos.
- **Correção:** dar prioridade ao nome (`min-width: 8ch; flex-shrink:0` ou badges com `flex-shrink` primeiro), esconder badges de menor valor abaixo de uma largura (container query) ou passá-los para uma 2.ª linha; mostrar `more`/`x` só em hover/foco/linha ativa (hoje `opacity:.5` permanente); pôr o nome do branch no `title`.
- **Esforço:** M. **Confiança:** média.

#### M2. Prioridade do dot do worktree esconde "à espera de ti" atrás de "stalled"
- **Local:** `agentState.ts:44-46` (`queued > stalled > waiting > working > awaiting > attention > idle`); `WorktreeRow.tsx:117-127`.
- **Evidência:** código. Um worktree com um agente `stalled` e outro `waiting` (precisa de resposta do utilizador) mostra só o laranja de "parado".
- **Impacto:** o estado mais acionável (waiting = pergunta/permissão pendente, o sino) fica oculto por um estado de diagnóstico; o utilizador só descobre ao abrir a lista de agentes. Também `working` ganha a `awaiting`, por isso um worktree com um agente ocupado esconde que outro espera por CI.
- **Correção:** `waiting` primeiro; considerar mostrar dot composto/contagem por estado (ex.: "2 working · 1 waiting") em vez de um só estado agregado.
- **Esforço:** S. **Confiança:** média (a prioridade pode ser intencional; confirmar com produto).

#### M3. Dots a 12px com glifo de 8px, traço 1.3: pouco legíveis e parecidos entre si
- **Local:** `StateDot.tsx:24-27` (`svg 8x8`, `stroke-width 1.3`), `agentState.ts:16-26` (paths), `styles.css:912-916`, `:594-596`, `:778`.
- **Evidência:** código. waiting `?` (vermelho, pulsa), stalled "ampulheta" (laranja), attention `!` (amarelo), awaiting `···` (ciano), idle `||` (cinza), working arco (roxo, roda). Vermelho/laranja/amarelo estão próximos em tons e os glifos têm ~5px úteis; o cor do glifo é `--on-accent` escuro sobre cores médias (contraste do glifo sobre `--stall`/`--await` por medir).
- **Impacto:** em monitores de baixa densidade/DPI 100% e para daltónicos (protanopia: vermelho vs laranja vs amarelo) distinguir waiting/stalled/attention depende de ler formas de 5px. A distinguibilidade é o objetivo da área e é o ponto frágil.
- **Correção:** dots de 14–16px nas linhas de agente (ou aumentar glifo), traço 1.6, e/ou cores com mais separação de luminosidade/matiz (ex.: stalled → magenta/rosa), medir contraste glifo/fundo ≥ 3:1 por estado nos dois temas. Opcional: texto curto ("espera", "parado") quando a sidebar é larga.
- **Esforço:** S–M. **Confiança:** média (precisa de screenshot para confirmar).

#### M4. Pesquisa dentro do terminal sem feedback de resultado
- **Local:** `terminal/useFind.ts:25-31`, `TerminalView.tsx:211-242`.
- **Evidência:** código. `findNext/findPrevious` não usam `onDidChangeResults` do SearchAddon; a barra não mostra "3/12" nem "sem resultados"; `incremental` só no input.
- **Impacto:** o utilizador carrega Enter sem saber se não há correspondências ou se o scroll não se moveu; em scrollback de 5000 linhas isto é frustrante. Também sem opções (maiúsculas/regex) e a barra (`right:18px`, 4 botões, input 180px) cobre texto no canto superior direito.
- **Correção:** ligar `search.onDidChangeResults` e mostrar contador / estado "sem resultados" (borda de erro); opcional toggle de caso.
- **Esforço:** S. **Confiança:** alta no código.

#### M5. Split de terminais sem limite mínimo útil e sem memória
- **Local:** `SplitPane.tsx:13,30-31` (`createSignal(0.5)`, clamp 0.2–0.8), `layout.ts:MIN_CENTER=360`.
- **Evidência:** código. `MIN_CENTER` garante 360px ao centro, mas o split parte esse centro: a 800px com sidebar 260 → centro 540 → 270px por terminal (~30 colunas), e a 20% ficam ~108px (~12 colunas). A fração é estado local e volta a 50% sempre que o split é reaberto.
- **Impacto:** em janelas pequenas o split dá terminais inutilizáveis (as linhas dos agentes partem a 30 colunas); o utilizador perde a proporção escolhida.
- **Correção:** clamp por largura em px (cada painel ≥ ~320px) e desativar/avisar o split se não couber; guardar a fração por sessão em `createPersisted`.
- **Esforço:** S. **Confiança:** média-alta.

#### M6. Clique direito no terminal faz copiar OU colar, sem menu nem pista
- **Local:** `terminal/useClipboard.ts:23-30`, `TerminalView.tsx:127`.
- **Evidência:** código. Sem seleção, clique direito cola imediatamente; falha de leitura do clipboard é engolida (`catch { }`, `:17`).
- **Impacto:** comportamento "Windows Terminal" é familiar a alguns, mas um clique direito acidental cola texto (possivelmente multi-linha) num agente/shell sem confirmação; em macOS/Linux o esperado é menu de contexto. Falha silenciosa quando o clipboard é imagem inválida ou o host recusa.
- **Correção:** menu de contexto (Copiar / Colar / Selecionar tudo / Limpar / Procurar), ou pelo menos aviso quando a colagem tem várias linhas; mostrar aviso quando `clipboard_read` falha.
- **Esforço:** M. **Confiança:** média (o comportamento pode ser desejado pelo dono).

### Baixo

#### B1. Sem filtro/pesquisa nem reordenação de projetos na sidebar
- **Local:** `Sidebar.tsx:281-359` (só `<For each={projects()}>`); sem input de filtro nem drag (`grep drag sidebar/*.tsx` vazio).
- **Impacto:** com 10+ projetos o único meio é dobrar/desdobrar; a paleta de comandos ajuda, mas a sidebar não. **Correção:** campo de filtro por nome/branch (Ctrl+K já existe: ligar) e ordenar por fixo/recente. **Esforço:** M. **Confiança:** alta.

#### B2. Tooltip do dot de worktree não diz porquê (awaiting/stalled/waiting)
- **Local:** `WorktreeRow.tsx:117` (não passa `tip`), contraste com `AgentList.tsx:154` (`tip={dotTip(a)}`).
- **Impacto:** o motivo de "awaiting" (`CI #3703`) só se vê na linha do agente; no worktree dobrado perde-se. **Correção:** propagar o `dotTip` do agente mais relevante. **Esforço:** S. **Confiança:** alta.

#### B3. Tempo ao lado do agente (`idle_ms`) mostrado também para agentes a trabalhar
- **Local:** `AgentList.tsx:29,194`: `a.running ? idleLabel(a.idle_ms) : T.ended`.
- **Impacto:** um agente "working" mostra um tempo "idle" que pode confundir (ou "agora"/0 s). Atualiza só quando a lista de sessões muda. **Correção:** esconder em `working`, ou rotular ("parado há 5 min") só em idle/stalled. **Esforço:** S. **Confiança:** baixa (semântica de `idle_ms` não verificada no daemon).

#### B4. Animações simultâneas por dot (pulso + rotação) escalam com o nº de agentes
- **Local:** `styles.css:593` (`.dot.work` pulse) + `:916` (`.dot.glyph.work svg` spin); `:778` (`.dot.wait` pulse).
- **Impacto:** cada agente a trabalhar anima dois elementos; com 20–30 agentes em sidebar são dezenas de animações contínuas (raster de box-shadow no waiting). Provável custo de CPU/GPU percebido. **Correção:** só uma animação por dot (a rotação do glifo, o pulso removido) e `animation-play-state: paused` quando a janela está sem foco. **Esforço:** S. **Confiança:** baixa (sem medições).

#### B5. Texto fixo em pt-PT em helpers de linha de agente
- **Local:** `agentRow.ts:19` (`em ${i.worktree}`) e `:25` (`Edita os mesmos ficheiros que:`), fora do dicionário `L`/`P` (AGENTS.md exige i18n tipado).
- **Impacto:** leitores de ecrã e tooltips ficam em pt-PT na localização en/pt-BR. **Correção:** mover para `L`. **Esforço:** S. **Confiança:** alta (verificar se `overlapTitle` deste ficheiro ainda é usado; `Sidebar.tsx` importa o de `rebase`).

---

### Top 5 da área
1. **A1** — rodapé/cabeçalho da sidebar rolam com a lista (fixar, só a lista com scroll).
2. **A2** — separador de redimensionar desalinhado a ≤1000px (limite `34vw` só no CSS).
3. **A3** — tab ativo pode ficar fora de vista; sem indicador de overflow nem menu de tabs.
4. **M1** — linha de worktree: nome do branch esmagado por badges/botões em sidebar estreita.
5. **M3/M2** — distinguibilidade dos estados: dots/glifos pequenos e agregação que esconde `waiting` atrás de `stalled`.

### Ideias de melhoria
- Cabeçalho da sidebar com filtro de projetos/worktrees (e estado agregado: "2 à espera, 1 parado") e atalho para saltar ao próximo agente em `waiting`.
- Vista compacta/detalhada comutável na sidebar (densidade) e container queries para esconder badges por largura.
- Mini-resumo de estado no cabeçalho de cada projeto dobrado (contagem por estado, com as mesmas formas dos dots).
- Menu "Tabs" (lista de todas as tabs com estado) + indicador de overflow; reordenar/fixar tabs.
- Menu de contexto do terminal e barra de pesquisa com contador; guardar tamanho do split por worktree.
- Correr `tools/e2e.mjs` adaptado a 800x600 e 1000x700 com 10 projetos de teste para fechar as lacunas de confiança (A2, M1, M3, B4).

---

## Ideias de melhoria transversais

- Corrida visual automática: `tools/visual.mjs` já existe; usá-lo para screenshots de regressão em claro/escuro, 800x600 e 3 locales, e referenciá-los aqui.
- Lint de design system: proibir sombras/cores/SVG inline fora de `ui/` e tokens (stylelint + eslint), como já se faz para i18n.
- Um componente único de confirmação para ações destrutivas/irreversíveis (pré-visualização do efeito, foco no campo, desfazer quando possível) reutilizado em Integrar, remover projeto/worktree, backup.
- Onboarding: transformar a linha de primeira execução nos 3 passos já escritos (hoje mortos) e detetar agentes instalados.
- Ver as secções "Ideias de melhoria" de cada área acima.
