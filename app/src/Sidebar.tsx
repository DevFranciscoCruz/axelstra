import { Icon, hasIcon } from "./Icon";
import { Select } from "./Select";
import { ask, askDiscard } from "./Confirm";
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { call, getSettings, detectPackageManagers, projectLogo, createWorktree, launchDefaultAgent, openInEditor, setProjectSetup, defaultShell, spawnSession, writeSession, removeProject, removeWorktree, killSession, removeSession, dispatchOverlaps, worktreeRename, worktreeOverview, worktreeSweep, worktreeSetParent } from "./api";
import { groupRunning, isFinished, layout, wtKey } from "./wtGroups";
import { Modal } from "./Modal";
import { pickProject } from "./projectActions";
import { StateDot } from "./StateDot";
import { ago } from "./time";
import { DOT_LABEL, dotOfSession, dotOfWorktree } from "./agentState";
import { agentRowInfo, agentRowLabel, overlapsOf, overlapTitle } from "./agentRow";
import { NoteView, openNote } from "./NoteView";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "./ContextMenu";
import { invoke } from "@tauri-apps/api/core";
import { press } from "./a11y";
import { S } from "./strings";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import logo from "./assets/logo.png";
import { Badge, Button, Field, Input, cn } from "./ui";
import { errorText } from "./appError";
import { waitShellReady } from "./shellReady";
import {
  AGENTS,
  canonAgent,
  setShowProjectSettings,
  agentKeys,
  isPinned,
  togglePin,
  setAttention,
  setShowAutomations,
  attention,
  sessionState,
  wtStatus,
  prByBranch,
  prKey,
  projectAgent,
  projectPrefsOf,
  projectTerse,
  setProjectPref,
  watchDispatch,
  dispatchTopics,
  confirmDiscard,
  closeSessionTab,
  creatingWorktree,
  setCreatingWorktree,
  removeRequest,
  setRemoveRequest,
  editorDirty,
  flowState,
  setEditorDirty,
  norm,
  showNotice,
  setShowDispatcher,
  setShowSettings,
  projects,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  selectedWorktree,
  sessions,
  sessionsOf,
  setError,
  activeSession,
  openSession,
  tabLabel,
  renameTab,
} from "./store";

function ProjectIcon(props: { id: string; name: string; url?: string | null }) {
  const [broken, setBroken] = createSignal(false);
  // a logo file inside the repo wins over the host avatar
  const [repoLogo] = createResource(() => props.id, projectLogo);
  const src = () => repoLogo() ?? props.url;
  return (
    <Show
      when={src() && !broken()}
      fallback={<span class="picon letter">{props.name.slice(0, 1).toUpperCase()}</span>}
    >
      <img class="picon" src={src()!} alt="" onError={() => setBroken(true)} />
    </Show>
  );
}

export function Sidebar() {
  // project id currently showing the "new worktree" dialog (in the store: the command palette opens it too)
  const creating = creatingWorktree;
  const setCreating = setCreatingWorktree;
  const [newName, setNewName] = createSignal("");
  // agent started in a new worktree: "none" or an AGENTS key, remembered between runs
  const NEWWT_KEY = "axelstra.newwt.agent";
  const [newAgent, setNewAgent] = createSignal<string>(
    (() => {
      try {
        const v = localStorage.getItem(NEWWT_KEY);
        return v === "none" ? v : v && AGENTS[v] ? canonAgent(v) : "claude";
      } catch {
        return "claude";
      }
    })(),
  );
  const setNewAgentSig = (v: string) => {
    setNewAgent(v);
    try {
      localStorage.setItem(NEWWT_KEY, v);
    } catch {
      /* preference only */
    }
  };
  // project id whose setup command is being edited
  // titles the dispatcher gave each worktree (axelstra dispatch), polled by the store while the sidebar is open
  onCleanup(watchDispatch());
  const topic = (path: string) => dispatchTopics()[norm(path)];
  // worktrees whose agent list is folded away
  const AGENTS_KEY = "axelstra.agentsHidden";
  const [agentsHidden, setAgentsHidden] = createSignal<ReadonlySet<string>>(
    (() => {
      try {
        return new Set<string>(JSON.parse(localStorage.getItem(AGENTS_KEY) ?? "[]"));
      } catch {
        return new Set<string>();
      }
    })(),
  );
  const toggleAgents = (path: string) => {
    const next = new Set(agentsHidden());
    const k = norm(path);
    if (!next.delete(k)) next.add(k);
    setAgentsHidden(next);
    try {
      localStorage.setItem(AGENTS_KEY, JSON.stringify([...next]));
    } catch {
      /* preference only */
    }
  };
  // worktrees whose branch is merged and clean, as the daemon's overview lists them (a few seconds old at most)
  const [mergedWts, setMergedWts] = createSignal<ReadonlySet<string>>(new Set());
  let overviewBusy = false;
  const loadMerged = () => {
    if (overviewBusy) return;
    overviewBusy = true;
    worktreeOverview()
      .then((ov) => setMergedWts(new Set(ov.flatMap((x) => x.merged.map((m) => wtKey(m.path))))), () => 0)
      .finally(() => (overviewBusy = false));
  };
  createEffect(() => {
    projects();
    loadMerged();
  });
  const overviewTimer = setInterval(() => !document.hidden && loadMerged(), 15000);
  onCleanup(() => clearInterval(overviewTimer));
  // parents whose children are folded away, and the projects whose "Concluídos" group is open
  const GROUPS_KEY = "axelstra.groupsFolded";
  const [folded, setFolded] = createSignal<ReadonlySet<string>>(
    (() => {
      try {
        return new Set<string>(JSON.parse(localStorage.getItem(GROUPS_KEY) ?? "[]"));
      } catch {
        return new Set<string>();
      }
    })(),
  );
  // a whole project folded to its header
  const projKey = (id: string) => `project:${id}`;
  const projFolded = (id: string) => folded().has(projKey(id));
  const toggleProject = (id: string) => {
    const next = new Set(folded());
    if (!next.delete(projKey(id))) next.add(projKey(id));
    setFolded(next);
    try {
      localStorage.setItem(GROUPS_KEY, JSON.stringify([...next]));
    } catch {
      /* preference only */
    }
  };
  const toggleFold = (path: string) => {
    const next = new Set(folded());
    const k = wtKey(path);
    if (!next.delete(k)) next.add(k);
    setFolded(next);
    try {
      localStorage.setItem(GROUPS_KEY, JSON.stringify([...next]));
    } catch {
      /* preference only */
    }
  };
  const [doneOpen, setDoneOpen] = createSignal<ReadonlySet<string>>(new Set());
  const toggleDone = (project: string) => {
    const next = new Set(doneOpen());
    if (!next.delete(project)) next.add(project);
    setDoneOpen(next);
  };
  const runningIn = (path: string) => sessionsOf(path).filter((s) => s.running).length;
  // worktrees that edit some of the same files (the daemon answers; until it can, the badge simply stays hidden)
  const [overlaps, setOverlaps] = createSignal<Record<string, string[]> | null>(null);
  const pollOverlaps = () => dispatchOverlaps().then(setOverlaps, () => setOverlaps(null));
  void pollOverlaps();
  const overlapTimer = setInterval(() => !document.hidden && void pollOverlaps(), 20000);
  onCleanup(() => clearInterval(overlapTimer));
  // agent being renamed in the list (F2 on its row), and the worktree being renamed
  const [renamingAgent, setRenamingAgent] = createSignal<number | null>(null);
  const [renamingWt, setRenamingWt] = createSignal<{ path: string; name: string } | null>(null);
  const [wtNewName, setWtNewName] = createSignal("");
  const [wtRenameErr, setWtRenameErr] = createSignal("");
  const finishAgentRename = (id: number, name: string | null) => {
    if (name !== null) renameTab(id, name);
    setRenamingAgent(null);
    queueMicrotask(() => document.querySelector<HTMLElement>(`[data-agent-row="${id}"]`)?.focus());
  };
  const submitWtRename = async () => {
    const r = renamingWt();
    const name = wtNewName().trim();
    if (!r || !name) return;
    try {
      await worktreeRename(r.path, name);
    } catch (e) {
      setWtRenameErr(String(e));
      return;
    }
    setRenamingWt(null);
    await refreshProjects();
    await refreshSessions();
    void pollOverlaps();
  };
  const idleLabel = (ms?: number) => (ms === undefined ? "" : ago(ms / 1000));
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");
  // package managers installed on this machine, offered as one-click setup commands
  const [pms, setPms] = createSignal<{ installed: string[]; suggested: string | null }>({ installed: [], suggested: null });
  createEffect(() => {
    const id = editSetup();
    if (!id) return;
    detectPackageManagers(id).then(setPms, () => setPms({ installed: [], suggested: null }));
  });

  // selecting a worktree never starts anything: agents and terminals come from the user or from restoring sessions
  const openWorktree = async (_projectId: string, path: string, _isMain: boolean) => {
    if (editorDirty()) {
      if (!(await confirmDiscard())) return;
      setEditorDirty(false);
    }
    selectWorktree(path);
  };

  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      setError(String(e));
    }
    await refreshProjects();
    await refreshSessions();
  };

  // a project with its own default agent starts new worktrees with it
  createEffect(() => {
    const id = creating();
    const own = id ? projectPrefsOf(id).agent : undefined;
    if (own && AGENTS[own]) setNewAgentSig(own);
  });

  const [pendingWt, setPendingWt] = createSignal<{ project: string; name: string } | null>(null);
  const submitWorktree = (projectId: string) =>
    run(async () => {
      const name = newName().trim();
      setCreating(null);
      setNewName("");
      if (!name) return;
      // git checks the files out, which takes a while in a big repo: show the row at once
      setPendingWt({ project: projectId, name });
      let w;
      try {
        w = await createWorktree(projectId, name);
      } finally {
        setPendingWt(null);
      }
      selectWorktree(w.path);
      const copied = w.env_copied?.length ? ` · ${w.env_copied.length} ficheiro(s) de ambiente copiado(s)` : "";
      const skipped = w.env_skipped?.length ? ` · ${w.env_skipped.length} ficou de fora` : "";
      showNotice(`Worktree "${name}" criado${copied}${skipped}.`);
      // the worktree exists by now: a failing agent must not skip the setup, and the user reads a sentence, not the raw error
      if (newAgent() !== "none") {
        try {
          await launchDefaultAgent(projectId, w.path, newAgent(), AGENTS[newAgent()], undefined, undefined, projectTerse(projectId));
        } catch (e) {
          showNotice(`O worktree "${name}" foi criado, mas o agente não arrancou (${errorText(e).slice(0, 120)}). Abre-o a partir do separador +.`);
        }
      }
      let setup = projects().find((p) => p.id === projectId)?.setup;
      if (!setup && (await getSettings().then((x) => x.auto_install, () => false))) {
        const pm = await detectPackageManagers(projectId).catch(() => null);
        if (pm?.package_json && pm.suggested) setup = `${pm.suggested} install`;
      }
      if (setup) {
        const [program, args] = await defaultShell();
        const { id } = await spawnSession(program, args, "setup", w.path, { project: projectId, worktree: w.path, kind: "shell" });
        // type only once the shell has printed its prompt (a slow profile can take seconds); if it never does, type anyway
        void waitShellReady(() => call<{ lines: string[] }>("session.screen", { id, lines: 5 }).then((r) => r.lines))
          .then(() => writeSession(id, setup + "\r"))
          .catch(() => 0);
      }
    });

  // leaving the new-worktree dialog with a name typed in asks first
  const closeNewWorktree = async () => {
    if (newName().trim() && !(await askDiscard("O nome do worktree ainda não foi criado."))) return;
    setCreating(null);
    setNewName("");
  };
  const closeSetup = async (setup?: string | null) => {
    if (setupText() !== (setup ?? "") && !(await askDiscard("O comando de setup tem alterações por guardar."))) return;
    setEditSetup(null);
  };

  const saveSetup = (projectId: string) =>
    run(async () => {
      await setProjectSetup(projectId, setupText());
      setEditSetup(null);
    });

  const copy = async (text: string) => {
    try {
      await invoke("clipboard_write", { text });
    } catch (e) {
      setError(String(e));
    }
  };

  // frees the memory of a worktree without deleting it: its terminals and agents are closed
  const sleepWorktree = async (path: string, name: string) => {
    const live = sessionsOf(path).filter((s) => s.running);
    if (live.length > 0) {
      const ok = await ask(`Pôr "${name}" a dormir?\n\nOs ${live.length} terminais e agentes a correr são fechados. O worktree e os ficheiros ficam como estão.`, {
        title: "Dormir",
        kind: "warning",
      });
      if (!ok) return;
    }
    await run(async () => {
      for (const s of sessionsOf(path)) {
        if (s.running) await killSession(s.id);
        await removeSession(s.id);
      }
    });
  };

    // "Mover para…": the other worktrees of the project, minus itself and what is already below it (that would loop)
  const moveMenu = (p: { id: string; worktrees: { path: string; branch: string | null; is_main: boolean; parent?: string | null }[] }, w: { path: string; parent?: string | null }, below: string[]): MenuItem[] => [
    { label: "Nenhum (nível superior)", icon: w.parent ? undefined : "check", run: () => run(() => worktreeSetParent(p.id, w.path, null)) },
    ...p.worktrees
      .filter((x) => !x.is_main && !below.includes(wtKey(x.path)))
      .map((x) => ({ label: x.branch ?? x.path, icon: wtKey(w.parent) === wtKey(x.path) ? "check" : undefined, run: () => run(() => worktreeSetParent(p.id, w.path, x.path)) })),
  ];

  const worktreeMenu = (p: { id: string; name: string; worktrees?: { path: string; branch: string | null; is_main: boolean; parent?: string | null }[] }, w: { path: string; branch: string | null; is_main: boolean; parent?: string | null }, label: string, below: string[] = []): MenuItem[] => [
    { label: "Atualizar", icon: "refresh", run: async () => { await refreshProjects(); await refreshSessions(); } },
    {
      label: S.replies,
      icon: "sparkle",
      children: [
        { label: "Seguir o global", icon: projectTerse(p.id) ? undefined : "check", run: () => setProjectPref(p.id, { terse: undefined }) },
        ...(["off", "short", "tiny"] as const).map((v) => ({ label: S.terse[v].label, icon: projectTerse(p.id) === v ? "check" : undefined, run: () => setProjectPref(p.id, { terse: v }) })),
      ],
    },
    { separator: true },
    {
      label: "Abrir em",
      icon: "folder",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(w.path)) },
        { label: "VS Code", run: () => run(() => openInEditor(w.path, "code")) },
      ],
    },
    { label: "Copiar caminho", icon: "copy", run: () => copy(w.path) },
    { label: "Mudar o nome do worktree…", icon: "pencil", disabled: w.is_main, run: () => { setWtNewName(w.branch ?? label); setWtRenameErr(""); setRenamingWt({ path: w.path, name: label }); } },
    { label: "Copiar nome do worktree", icon: "copy", run: () => copy(w.branch ?? label) },
    { label: "Mover para…", icon: "branch", disabled: w.is_main || !p.worktrees || p.worktrees.filter((x) => !x.is_main).length < 2, children: p.worktrees ? moveMenu({ id: p.id, worktrees: p.worktrees }, w, below) : [] },
    { separator: true },
    { label: isPinned(w.path) ? "Desafixar" : "Fixar", icon: "pin", run: () => togglePin(w.path) },
    { label: "Marcar como não lido", icon: "circleHalf", run: () => setAttention((cur) => new Set(cur).add(norm(w.path))) },
    { separator: true },
    { label: "Dormir", icon: "moon", run: () => sleepWorktree(w.path, label) },
    { label: "Remover worktree", icon: "x", danger: true, disabled: w.is_main, run: () => dropWorktree(p.id, w.path, label) },
  ];

  // the existing sweep removes every merged worktree, of all projects: the question says so
  const cleanFinished = async (names: string[]) => {
    const ok = await ask(
      `Remover os worktrees concluídos (${names.length})?\n\n${names.join("\n")}\n\nA limpeza remove todos os worktrees já merged, de todos os projetos.`,
      { title: "Limpar concluídos", kind: "danger", okLabel: "Remover", cancelLabel: "Cancelar" },
    );
    if (!ok) return;
    try {
      const done = (await worktreeSweep()).filter((x) => x.removed).length;
      showNotice(done === 1 ? "1 worktree removida" : `${done} worktrees removidas`);
    } catch (e) {
      setError(String(e));
    }
    await refreshProjects();
    await refreshSessions();
    loadMerged();
  };

  const dropProject = async (id: string, name: string) => {
    const ok = await ask(`Tirar "${name}" da lista?

Os ficheiros e worktrees não são apagados.`, { title: "Tirar projeto", kind: "warning", okLabel: "Tirar", cancelLabel: "Cancelar" });
    if (ok) {
      let gone = true;
      await run(async () => {
        try {
          await removeProject(id);
        } catch (e) {
          gone = false;
          throw e;
        }
      });
      if (gone) showNotice(S.projectRemoved(name));
    }
  };

  const projectMenu = (p: { id: string; name: string; path: string; setup?: string | null }): MenuItem[] => [
    { label: "Novo worktree", icon: "plus", run: () => setCreating(p.id) },
    { label: "Comando de setup…", icon: "gear", run: () => { setSetupText(p.setup ?? ""); setEditSetup(p.id); } },
    {
      label: "Agente por omissão",
      icon: "sparkle",
      children: [
        { label: "Seguir o global", icon: projectPrefsOf(p.id).agent ? undefined : "check", run: () => setProjectPref(p.id, { agent: undefined }) },
        ...agentKeys().map((a) => ({ label: projectPrefsOf(p.id).agent && canonAgent(projectPrefsOf(p.id).agent!) === a ? `${a} ✓` : a, icon: hasIcon(a) ? a : undefined, run: () => setProjectPref(p.id, { agent: a }) })),
      ],
    },
    { separator: true },
    {
      label: "Abrir em",
      icon: "folder",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(p.path)) },
      ],
    },
    { label: "Copiar caminho", icon: "copy", run: () => copy(p.path) },
    { separator: true },
    { label: "Tirar projeto da lista", icon: "x", danger: true, run: () => dropProject(p.id, p.name) },
  ];

  // worktrees being removed: a second click (or menu entry) while it runs is ignored
  const removing = new Set<string>();
  const dropWorktree = async (projectId: string, path: string, name: string) => {
    if (removing.has(norm(path))) return showNotice(`A remover "${name}"…`);
    // what would be lost, so the question is not a blind "are you sure"
    const st = wtStatus()[norm(path)];
    const risks: string[] = [];
    if (st?.dirty) risks.push(`${st.dirty} ${st.dirty === 1 ? "ficheiro com alterações" : "ficheiros com alterações"} por guardar`);
    if (st && st.ahead > 0) risks.push(st.upstream ? `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} por enviar` : `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} que só existem aqui (sem remoto)`);
    const live = sessionsOf(path).filter((x) => x.running).length;
    if (live) risks.push(`${live} ${live === 1 ? "terminal ou agente a correr" : "terminais ou agentes a correr"}`);
    const lost = risks.length ? `\n\nVais perder: ${risks.join("; ")}.` : "\n\nNão há alterações nem commits por enviar.";
    const ok = await ask(
      `Remover o worktree "${name}"?${lost}\n\nA pasta e o branch são apagados e os terminais dele são fechados.`,
      { title: "Remover worktree", kind: "danger" },
    );
    if (!ok || removing.has(norm(path))) return;
    removing.add(norm(path));
    showNotice(`A remover "${name}"…`);
    try {
      let gone = true;
      await run(async () => {
        try {
          for (const s of sessionsOf(path)) {
            if (s.running) await killSession(s.id);
            await removeSession(s.id);
          }
          await removeWorktree(projectId, path);
        } catch (e) {
          gone = false;
          throw e;
        }
        if (norm(selectedWorktree()) === norm(path)) selectWorktree(null);
      });
      if (gone) showNotice(S.worktreeRemoved(name));
    } finally {
      removing.delete(norm(path));
    }
  };

  createEffect(() => {
    const r = removeRequest();
    if (!r) return;
    setRemoveRequest(null);
    void dropWorktree(r.project, r.path, r.name);
  });

  return (
    <aside class="sidebar">
      <div class="brand">
        <img src={logo} alt="" />
        Axelstra
      </div>
      <Button variant="default" onClick={() => void pickProject()}>
        + Adicionar projeto
      </Button>

      <div class="section">Projetos</div>
      <For
        each={projects()}
        fallback={
          <div class="empty">
            <div>Nenhum projeto</div>
            <Button variant="default" onClick={() => void pickProject()}>Adicionar o primeiro projeto</Button>
          </div>
        }
      >
        {(p) => {
          // main first, then the tree of parents and children; the finished ones are listed apart
          const lay = createMemo(() => {
            const fin = new Set(p.worktrees.filter((w) => isFinished(w, mergedWts(), runningIn(w.path))).map((w) => wtKey(w.path)));
            return layout(p.worktrees, { finished: fin, collapsed: folded(), pinned: isPinned });
          });
          return (
          <div class="project" classList={{ collapsed: projFolded(p.id) }}>
            <div
              class="project-head"
              title={p.path}
              onContextMenu={(e) => showMenu(e, projectMenu(p))}
              onKeyDown={(e) => {
                // the menu key and Shift+F10 open the project menu from any button of the header
                if (!isMenuKey(e)) return;
                e.preventDefault();
                const r = e.currentTarget.getBoundingClientRect();
                showMenuAt(r.left + 24, r.bottom, projectMenu(p));
              }}
            >
              <Button
                type="button"
                variant="icon"
                size="icon"
                class="size-5 flex-none opacity-80"
                aria-expanded={!projFolded(p.id)}
                aria-label={projFolded(p.id) ? `Abrir o projeto ${p.name}` : `Fechar o projeto ${p.name}`}
                title={projFolded(p.id) ? "Mostrar os worktrees" : "Esconder os worktrees"}
                onClick={() => toggleProject(p.id)}
              >
                <Icon name={projFolded(p.id) ? "chevRight" : "chevDown"} size={12} />
              </Button>
              <ProjectIcon id={p.id} name={p.name} url={p.icon_url} />
              <span class="pname" title={p.slug ?? p.path}>{p.name}</span>
              <Button variant="icon" size="glyph" title="Novo worktree" aria-label="Novo worktree" onClick={() => setCreating(creating() === p.id ? null : p.id)}>
                <Icon name="plus" />
              </Button>
              <Button
                variant="icon"
                size="glyph"
                title={p.setup ? `Setup: ${p.setup}` : "Comando de setup para novos worktrees"} aria-label={p.setup ? `Setup: ${p.setup}` : "Comando de setup para novos worktrees"}
                onClick={() => {
                  setSetupText(p.setup ?? "");
                  setEditSetup(editSetup() === p.id ? null : p.id);
                }}
              >
                <Icon name="gear" />
              </Button>
              <Button variant="icon" size="glyph" title="Tirar da lista (não apaga ficheiros)" aria-label="Tirar projeto da lista" onClick={() => void dropProject(p.id, p.name)}>
                <Icon name="x" />
              </Button>
            </div>
            <Show when={editSetup() === p.id}>
              <Input
                class="mx-1.5 mb-1.5 mt-0.5 w-[calc(100%-12px)] border-primary bg-background"
                placeholder="setup, ex.: bun install (vazio = nenhum)"
                aria-label={`Comando de setup do projeto ${p.name}`}
                value={setupText()}
                onInput={(e) => setSetupText(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void saveSetup(p.id);
                  if (e.key === "Escape") void closeSetup(p.setup);
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <Show when={pms().installed.length > 0}>
                <div class="pm-row">
                  <For each={pms().installed}>
                    {(m) => (
                      <Button
                        variant="secondary"
                        size="pill"
                        class={cn("border-border px-2.5 py-0.5 text-sm", setupText() === `${m} install` && "border-primary text-primary")}
                        title={m === pms().suggested ? "Sugerido pelo lockfile do projeto" : `Usar ${m} install`}
                        onClick={() => setSetupText(`${m} install`)}
                      >
                        {m}
                        {m === pms().suggested ? <Icon name="star" size={10} inline /> : null}
                      </Button>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
            <Show when={pendingWt()?.project === p.id}>
              <div class="item wt pending">
                <span class="dot work" />
                <span class="name">A criar {pendingWt()!.name}…</span>
              </div>
            </Show>
            <For each={lay().rows.map((r) => r.wt)}>
              {(w) => {
                const ri = () => lay().rows.find((r) => wtKey(r.wt.path) === wtKey(w.path));
                const depth = () => ri()?.depth ?? 0;
                const kids = () => ri()?.kids ?? 0;
                const isFolded = () => kids() > 0 && folded().has(wtKey(w.path));
                // running agents of the whole group, shown on the parent row while its children are folded away
                const groupLive = () => groupRunning(ri()?.members ?? [], (k) => runningIn(k));
                const label = () => w.branch ?? "(destacado)";
                const live = () => sessionsOf(w.path).filter((s) => s.running).length;
                const working = () => sessionsOf(w.path).some((s) => sessionState(s) === "working");
                const waiting = () => sessionsOf(w.path).some((s) => sessionState(s) === "waiting");
                const needs = () => attention().has(norm(w.path));
                const queued = () => live() === 0 && flowState()[norm(w.path)] === "queued";
                const st = () => wtStatus()[norm(w.path)];
                const pr = () => prByBranch()[prKey(p.id, w.branch)];
                // agents of this worktree; workers that an agent of another worktree started show in their own worktree
                const agents = () => sessionsOf(w.path).filter((x) => x.meta?.kind === "agent");
                return (
                  <>
                  <div
                    class="item wt"
                    classList={{ active: norm(selectedWorktree()) === norm(w.path), child: depth() > 0 }}
                    style={depth() > 0 ? { "margin-left": `${depth() * 14}px` } : undefined}
                    onKeyDown={(e) => {
                      // the menu key and Shift+F10 open the row menu from the keyboard
                      if (isMenuKey(e)) {
                        e.preventDefault();
                        const r = e.currentTarget.getBoundingClientRect();
                        showMenuAt(r.left + 24, r.bottom, worktreeMenu(p, w, label(), ri()?.members));
                        return;
                      }
                      // left folds a group, right opens it
                      if (kids() > 0 && (e.target as HTMLElement).classList.contains("wt-open") && ((e.key === "ArrowLeft" && !isFolded()) || (e.key === "ArrowRight" && isFolded()))) {
                        e.preventDefault();
                        toggleFold(w.path);
                      }
                    }}
                    onContextMenu={(e) => showMenu(e, worktreeMenu(p, w, label(), ri()?.members))}
                    title={w.path}
                  >
                    <Show when={kids() > 0}>
                      <Button
                        type="button"
                        variant="link"
                        size="bare"
                        class="h-[18px] w-4 flex-none rounded-sm"
                        aria-expanded={!isFolded()}
                        aria-label={isFolded() ? `Abrir o grupo de ${label()}` : `Fechar o grupo de ${label()}`}
                        title={isFolded() ? "Mostrar os worktrees do grupo" : "Esconder os worktrees do grupo"}
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleFold(w.path);
                        }}
                      >
                        <Icon name={isFolded() ? "chevRight" : "chevDown"} size={11} />
                      </Button>
                    </Show>
                    <Button type="button" variant="row" size="bare" class="min-w-0 flex-1 gap-2" aria-current={norm(selectedWorktree()) === norm(w.path) ? "true" : undefined} onClick={() => void openWorktree(p.id, w.path, w.is_main)}>
                    <StateDot state={dotOfWorktree({ queued: queued(), waiting: waiting(), working: working(), needs: needs(), live: live() })} />
                    <span class="name">{w.is_main ? (p.default_branch ?? "main") : label()}</span>
                    <Show when={!w.is_main && topic(w.path)?.title && topic(w.path)!.title !== label()}>
                      <span class="wt-topic" title={topic(w.path)!.title}>{topic(w.path)!.title}</span>
                    </Show>
                    <Show when={topic(w.path)?.archived}>
                      <Badge variant="quiet" title="Arquivado: o despachante já não envia pedidos para aqui">arquivado</Badge>
                    </Show>
                    <Show when={w.is_main && w.branch !== (p.default_branch ?? "main")}>
                      <span class="off-main" title={`O worktree principal está em "${label()}", não em ${p.default_branch ?? "main"}`}><Icon name="warning" size={12} /></span>
                    </Show>
                    <Show when={st()?.dirty}>
                      <Badge variant="quiet" title={`${st()!.dirty} ficheiro(s) com alterações por guardar`}><i class="size-1.5 flex-none rounded-full bg-current opacity-70" />{st()!.dirty}</Badge>
                    </Show>
                    <Show when={overlapTitle(overlapsOf(overlaps(), w.path))}>
                      {(t) => (
                        <Badge variant="overlap" role="img" title={t()} aria-label={t()}><Icon name="warning" size={11} /></Badge>
                      )}
                    </Show>
                    <Show when={st()?.ahead || st()?.behind}>
                      <Badge variant="quiet" title={`${st()!.ahead} commit(s) à frente e ${st()!.behind} atrás do remoto`}>
                        {st()!.ahead ? <span class="inline-flex items-center gap-px"><Icon name="arrowUp" size={10} />{st()!.ahead}</span> : null}
                        {st()!.behind ? <span class="inline-flex items-center gap-px"><Icon name="arrowDown" size={10} />{st()!.behind}</span> : null}
                      </Badge>
                    </Show>
                    <Show when={pr()}>
                      <Badge
                        variant={pr()!.checks === "pass" ? "prOk" : pr()!.checks === "fail" ? "prErr" : pr()!.checks === "pending" ? "prPend" : "pr"}
                        title={`PR #${pr()!.number}: ${pr()!.title}${pr()!.checks ? ` (CI: ${pr()!.checks})` : ""}`}
                      >
                        #{pr()!.number}
                        {pr()!.checks === "pass" ? <Icon name="check" size={10} inline /> : pr()!.checks === "fail" ? <Icon name="x" size={10} inline /> : pr()!.checks === "pending" ? <Icon name="ellipsis" size={10} inline /> : null}
                      </Badge>
                    </Show>
                    <Show when={isFolded() && groupLive() > 0} fallback={<Show when={live() > 0}><span class="count">{live()}</span></Show>}>
                      <span class="count group" title={`${groupLive()} a correr neste grupo (${kids()} ${kids() === 1 ? "worktree" : "worktrees"} dentro)`}>{groupLive()}</span>
                    </Show>
                    <Show when={isPinned(w.path)}>
                      <span class="pin" title="Fixado"><Icon name="pin" size={11} /></span>
                    </Show>
                    </Button>
                    <Button
                      variant="icon"
                      size="glyph"
                      class="x"
                      title="Mais ações" aria-label="Mais ações"
                      onClick={(e) => {
                        e.stopPropagation();
                        const r = e.currentTarget.getBoundingClientRect();
                        showMenuAt(r.left, r.bottom + 2, worktreeMenu(p, w, label(), ri()?.members));
                      }}
                    >
                      <Icon name="more" />
                    </Button>
                    <Show when={!w.is_main}>
                      <Button
                        variant="icon"
                        size="glyph"
                        class="x"
                        title="Remover worktree" aria-label="Remover worktree"
                        onClick={(e) => {
                          e.stopPropagation();
                          void dropWorktree(p.id, w.path, label());
                        }}
                      >
                        <Icon name="x" />
                      </Button>
                    </Show>
                  </div>
                  <Show when={agents().length > 0}>
                    <div class="wt-agents" style={depth() > 0 ? { "margin-left": `${14 + depth() * 14}px` } : undefined}>
                      <Button variant="link" size="bare" class="w-full justify-between px-1.5 pb-1 pt-0.5 text-xs" onClick={() => toggleAgents(w.path)} aria-expanded={!agentsHidden().has(norm(w.path))}>
                        <span>{agents().length} {agents().length === 1 ? "agente" : "agentes"}</span>
                        <Icon name={agentsHidden().has(norm(w.path)) ? "chevRight" : "chevDown"} size={11} />
                      </Button>
                      <Show when={!agentsHidden().has(norm(w.path))}>
                        <For each={agents()}>
                          {(a) => {
                            const info = () => agentRowInfo(a, w.path, tabLabel(a));
                            const state = () => DOT_LABEL[dotOfSession(a)];
                            const active = () => activeSession() === a.id && norm(selectedWorktree()) === norm(w.path);
                            return (
                              <div class="wt-agent" classList={{ active: active() }}>
                                <Show
                                  when={renamingAgent() !== a.id}
                                  fallback={
                                    <Input
                                      class="mx-1 my-0.5 h-auto min-w-0 flex-1 px-1.5 py-[3px] text-sm"
                                      aria-label="Nome do agente"
                                      value={info().name}
                                      ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
                                      onKeyDown={(e) => {
                                        if (e.key === "Enter") finishAgentRename(a.id, e.currentTarget.value);
                                        else if (e.key === "Escape") finishAgentRename(a.id, null);
                                      }}
                                      onBlur={(e) => renamingAgent() === a.id && finishAgentRename(a.id, e.currentTarget.value)}
                                    />
                                  }
                                >
                                  <Button
                                    type="button"
                                    variant="row"
                                    size="bare"
                                    class="min-w-0 flex-1 gap-2 px-1.5 py-1"
                                    data-agent-row={a.id}
                                    aria-label={agentRowLabel(info(), state())}
                                    aria-current={active() ? "true" : undefined}
                                    title={`${info().name} (F2 muda o nome)`}
                                    onClick={() => openSession(a.meta?.worktree ?? w.path, a.id)}
                                    onMouseDown={(e) => e.button === 1 && e.preventDefault()}
                                    onAuxClick={(e) => {
                                      if (e.button !== 1) return;
                                      e.preventDefault();
                                      void closeSessionTab(a);
                                    }}
                                    onKeyDown={(e) => {
                                      if (e.key === "F2") {
                                        e.preventDefault();
                                        setRenamingAgent(a.id);
                                      }
                                    }}
                                  >
                                    <StateDot state={dotOfSession(a)} />
                                    <span class="wt-agent-name">{a.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}{info().name}</span>
                                    <Show when={info().kind}>
                                      <span class="wt-agent-kind inline-flex items-center text-muted-foreground" title={info().kind} role="img" aria-label={`Agente ${info().kind}`}>{hasIcon(info().kind) ? <Icon name={info().kind} size={13} /> : info().kind}</span>
                                    </Show>
                                    <Show when={info().worktree}>
                                      <span class="wt-agent-wt inline-flex items-center text-muted-foreground" title={info().worktree} role="img" aria-label={`Worktree ${info().worktree}`}><Icon name="branch" size={12} /></span>
                                    </Show>
                                  </Button>
                                </Show>
                                <Show when={info().hasReport && renamingAgent() !== a.id}>
                                  <Button type="button" variant="chip" size="chip" class="mr-1 flex-none px-[6px]" aria-label={`Relatório de ${info().name}`} title="Abrir o relatório deste agente" onClick={() => openNote(a.id, info().name)}>
                                    <Icon name="file" size={12} />
                                  </Button>
                                </Show>
                                <span class="wt-agent-time">{a.running ? idleLabel(a.idle_ms) : "fim"}</span>
                              </div>
                            );
                          }}
                        </For>
                      </Show>
                    </div>
                  </Show>
                  </>
                );
              }}
            </For>
            <Show when={lay().finished.length > 0}>
              <div class="wt-done">
                <div class="wt-done-head">
                  <Button type="button" variant="link" size="bare" class="gap-[5px] py-[3px] text-xs" aria-expanded={doneOpen().has(p.id)} onClick={() => toggleDone(p.id)}>
                    <Icon name={doneOpen().has(p.id) ? "chevDown" : "chevRight"} size={11} />
                    <span>Concluídos ({lay().finished.length})</span>
                  </Button>
                  <Button type="button" variant="chip" size="chip" class="py-px" title="Remover os worktrees já merged" onClick={() => void cleanFinished(lay().finished.map((w) => `${p.name}: ${w.branch ?? w.path}`))}>
                    Limpar
                  </Button>
                </div>
                <Show when={doneOpen().has(p.id)}>
                  <For each={lay().finished}>
                    {(w) => {
                      const label = () => w.branch ?? "(destacado)";
                      return (
                        <div class="item wt finished" classList={{ active: norm(selectedWorktree()) === norm(w.path) }} onContextMenu={(e) => showMenu(e, worktreeMenu(p, w, label()))} title={w.path}>
                          <Button type="button" variant="row" size="bare" class="min-w-0 flex-1 gap-2" aria-current={norm(selectedWorktree()) === norm(w.path) ? "true" : undefined} onClick={() => void openWorktree(p.id, w.path, w.is_main)}>
                            <StateDot state="done" />
                            <span class="name">{label()}</span>
                          </Button>
                          <Button
                            variant="icon"
                            size="glyph"
                            class="x"
                            title="Remover worktree" aria-label={`Remover ${label()}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              void dropWorktree(p.id, w.path, label());
                            }}
                          >
                            <Icon name="x" />
                          </Button>
                        </div>
                      );
                    }}
                  </For>
                </Show>
              </div>
            </Show>
          </div>
          );
        }}
      </For>

      <div class="grow" />
      <Button variant="link" size="bare" class="justify-start gap-1.5 rounded-md px-2.5 py-2 text-sm hover:bg-foreground/5" onClick={() => setShowAutomations(true)}>
        <Icon name="clock" size={13} /> Automatizações
      </Button>
      <Button variant="link" size="bare" class="justify-start gap-1.5 rounded-md px-2.5 py-2 text-sm hover:bg-foreground/5" onClick={() => setShowDispatcher(true)} title="Para onde foi cada pedido feito na main">
        <Icon name="branch" size={13} /> Despachante
      </Button>
      <Button variant="link" size="bare" class="justify-start gap-1.5 rounded-md px-2.5 py-2 text-sm hover:bg-foreground/5" onClick={() => setShowSettings(true)}>
        <Icon name="gear" size={13} /> Definições
      </Button>
      <Show when={creating()}>
        {(pid) => {
          const proj = () => projects().find((x) => x.id === pid());
          return (
            <Modal label="Novo worktree" dirty={!!newName().trim()} onClose={() => void closeNewWorktree()}>
              <div class="m-title" style={{ "font-weight": "600" }}>Novo worktree</div>
              <Field label="Projeto" full><Input value={proj()?.name ?? ""} disabled /></Field>
              <Field label="Nome" full><Input
                  placeholder="nome do worktree (também o nome do branch)"
                  value={newName()}
                  onInput={(e) => setNewName(e.currentTarget.value)}
                  onKeyDown={(e) => e.key === "Enter" && newName().trim() && void submitWorktree(pid())}
                  ref={(el) => queueMicrotask(() => el.focus())}
                /></Field>
              <Field label="Agente" full><Select
                  value={newAgent()}
                  label="Agente"
                  iconOnly
                  options={[{ value: "none", label: "Nenhum (só o worktree)" }, ...agentKeys().map((a) => ({ value: a, label: a, icon: hasIcon(a) ? a : undefined }))]}
                  onChange={setNewAgentSig}
                /></Field>
              <div class="m-actions" style={{ "justify-content": "flex-end" }}>
                <Button onClick={() => void closeNewWorktree()}>Cancelar</Button>
                <Button variant="default" disabled={!newName().trim()} onClick={() => void submitWorktree(pid())}>
                  Criar worktree
                </Button>
              </div>
            </Modal>
          );
        }}
      </Show>
      <NoteView />
      <Show when={renamingWt()}>
        {(r) => (
          <Modal label="Mudar o nome do worktree" dirty={wtNewName().trim() !== r().name} onClose={() => setRenamingWt(null)}>
            <div class="m-title" style={{ "font-weight": "600" }}>Mudar o nome do worktree</div>
            <Field label="Nome" full><Input
                value={wtNewName()}
                onInput={(e) => { setWtNewName(e.currentTarget.value); setWtRenameErr(""); }}
                onKeyDown={(e) => e.key === "Enter" && void submitWtRename()}
                ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
              /></Field>
            <Show when={wtRenameErr()}>
              <div class="note-status bad" role="alert">{wtRenameErr()}</div>
            </Show>
            <div class="m-actions" style={{ "justify-content": "flex-end" }}>
              <Button onClick={() => setRenamingWt(null)}>Cancelar</Button>
              <Button variant="default" disabled={!wtNewName().trim() || wtNewName().trim() === r().name} onClick={() => void submitWtRename()}>
                Mudar o nome
              </Button>
            </div>
          </Modal>
        )}
      </Show>
    </aside>
  );
}
