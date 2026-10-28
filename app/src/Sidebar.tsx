import { createLive } from "./live";
import { askDiscard } from "./Confirm";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import { getSettings, detectPackageManagers, createWorktree, launchDefaultAgent, listSessions, setProjectSetup, dispatchOverlapFiles, worktreeRename } from "./api";
import { leadOf } from "./leadRole";
import { groupRunning, isFinished, layout, wtKey } from "./wtGroups";
import { ProjectHead, SetupEditor } from "./sidebar/ProjectHead";
import { CreateWorktree } from "./sidebar/CreateWorktree";
import { RenameWorktree } from "./sidebar/RenameWorktree";
import { FinishedRow, WorktreeRow } from "./sidebar/WorktreeRow";
import { createSidebarActions } from "./sidebar/actions";
import { MenuDeps, projectMenu, worktreeMenu } from "./sidebar/menus";
import { overlapCount, overlapEntries, overlapTitle, type OverlapFiles } from "./rebase";
import { NoteView } from "./NoteView";
import { S } from "./strings";
import { P } from "./strings.panels";
import logo from "./assets/logo.png";
import "./sidetabs.css";
import { Button, Input } from "./ui";
import { Icon } from "./Icon";
import { errorText } from "./appError";
import { isMultiLine, runSetup } from "./setupRun";
import { norm, toggleIn } from "./paths";
import {
  AGENTS,
  canonAgent,
  isPinned,
  setShowAutomations,
  projectPrefsOf,
  projectTerse,
  watchDispatch,
  dispatchTopics,
  confirmDiscard,
  creatingWorktree,
  setCreatingWorktree,
  removeRequest,
  setRemoveRequest,
  editorDirty,
  setEditorDirty,
  showNotice,
  setShowProjectSettings,
  setShowDispatcher,
  setShowAddProject,
  setShowSettings,
  projects,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  selectIfStill,
  openIfStill,
  navStamp,
  sessionsOf,
  overview,
  refreshOverview,
  watchOverview,
} from "./store";
import { createPersisted, isStringArray } from "./persist";

const T = P.sidebar;
const NEWWT_KEY = "axelstra.newwt.agent";
const AGENTS_KEY = "axelstra.agentsHidden";
const GROUPS_KEY = "axelstra.groupsFolded";

export function Sidebar() {
  // project id currently showing the "new worktree" dialog (in the store: the command palette opens it too)
  const creating = creatingWorktree;
  const setCreating = setCreatingWorktree;
  const [newName, setNewName] = createSignal("");
  // agent started in a new worktree: "none" or an AGENTS key, remembered between runs
  const [savedAgent, setSavedAgent] = createPersisted<string>(NEWWT_KEY, (v): v is string => typeof v === "string", "claude");
  const [newAgent, setNewAgent] = createSignal<string>(savedAgent() === "none" ? "none" : AGENTS[savedAgent()] ? canonAgent(savedAgent()) : "claude");
  const setNewAgentSig = (v: string) => {
    setNewAgent(v);
    setSavedAgent(v);
  };
  // status dots stop animating while the window has no focus (css: html[data-blur])
  const syncBlur = () => document.documentElement.toggleAttribute("data-blur", !document.hasFocus());
  window.addEventListener("blur", syncBlur);
  window.addEventListener("focus", syncBlur);
  onCleanup(() => {
    window.removeEventListener("blur", syncBlur);
    window.removeEventListener("focus", syncBlur);
  });
  const actions = createSidebarActions();
  const { run, cleanFinished, dropProject, dropWorktree } = actions;

  // titles the dispatcher gave each worktree (axelstra dispatch), polled by the store while the sidebar is open
  onCleanup(watchDispatch());
  const topic = (path: string) => dispatchTopics()[norm(path)];
  // worktrees whose agent list is folded away
  const [agentsHiddenList, setAgentsHiddenList] = createPersisted(AGENTS_KEY, isStringArray, [] as string[]);
  const agentsHidden = createMemo<ReadonlySet<string>>(() => new Set(agentsHiddenList()));
  // worktrees whose branch is merged and clean, as the daemon's overview lists them (a few seconds old at most)
  const mergedWts = createMemo<ReadonlySet<string>>(() => new Set(overview().flatMap((x) => x.merged.map((m) => wtKey(m.path)))));
  const loadMerged = () => void refreshOverview();
  onCleanup(watchOverview());
  createEffect(() => {
    projects();
    loadMerged();
  });
  // parents whose children are folded away, and whole projects folded to their header
  const [foldedList, setFoldedList] = createPersisted(GROUPS_KEY, isStringArray, [] as string[]);
  const folded = createMemo<ReadonlySet<string>>(() => new Set(foldedList()));
  const projKey = (id: string) => `project:${id}`;
  const projFolded = (id: string) => folded().has(projKey(id));
  const toggleFolded = (key: string) => setFoldedList([...toggleIn(folded(), key)]);
  // the projects whose "Concluídos" group is open
  const [doneOpen, setDoneOpen] = createSignal<ReadonlySet<string>>(new Set());
  const runningIn = (path: string) => sessionsOf(path).filter((s) => s.running).length;
  // worktrees that edit some of the same files (the daemon answers; until it can, the badge simply stays hidden)
  const [overlaps, setOverlaps] = createSignal<OverlapFiles | null>(null);
  const pollOverlaps = () => dispatchOverlapFiles().then(setOverlaps, () => setOverlaps(null));
  void pollOverlaps();
  // who edits what changes with the files, the worktrees and the dispatcher's plan
  const overlapLive = createLive(pollOverlaps, ["fs.changed", "projects.changed", "dispatch.changed"]);
  onCleanup(overlapLive.stop);

  // the worktree being renamed
  const [renamingWt, setRenamingWt] = createSignal<{ path: string; name: string } | null>(null);
  const [wtNewName, setWtNewName] = createSignal("");
  const [wtRenameErr, setWtRenameErr] = createSignal("");
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

  // project whose setup command is being edited, and the package managers installed here, offered as one-click commands
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");
  const [pms, setPms] = createSignal<{ installed: string[]; suggested: string | null }>({ installed: [], suggested: null });
  createEffect(() => {
    const id = editSetup();
    if (!id) return;
    detectPackageManagers(id).then(setPms, () => setPms({ installed: [], suggested: null }));
  });

  // selecting a worktree never starts anything: agents and terminals come from the user or from restoring sessions
  const openWorktree = async (path: string) => {
    if (editorDirty()) {
      if (!(await confirmDiscard())) return;
      setEditorDirty(false);
    }
    selectWorktree(path);
  };

  // a worktree without a lead starts one: the project's default agent, which the daemon names "main"
  const startingLead = new Set<string>();
  const startLead = async (projectId: string, path: string, name: string) => {
    if (startingLead.has(norm(path))) return;
    startingLead.add(norm(path));
    const stamp = navStamp();
    try {
      // the sidebar may be showing an old list: ask the daemon, and never start a second lead beside a running one
      const live = leadOf(await listSessions(), path);
      if (live) {
        await refreshSessions();
        openIfStill(stamp, path, live.id);
        return;
      }
      const agent = projectPrefsOf(projectId).agent;
      const key = agent && AGENTS[agent] ? agent : canonAgent(savedAgent() === "none" ? "claude" : savedAgent());
      const id = await launchDefaultAgent(projectId, path, key, AGENTS[key], undefined, undefined, projectTerse(projectId));
      await refreshSessions();
      openIfStill(stamp, path, id);
    } catch (e) {
      showNotice(T.leadFailed(name, errorText(e).slice(0, 120)));
    } finally {
      startingLead.delete(norm(path));
    }
  };

  // a project with its own default agent starts new worktrees with it
  createEffect(() => {
    const id = creating();
    const own = id ? projectPrefsOf(id).agent : undefined;
    if (own && AGENTS[own]) setNewAgentSig(own);
  });

  const [pendingWt, setPendingWt] = createSignal<{ project: string; name: string } | null>(null);
  const submitWorktree = (projectId: string) =>
    // eslint-disable-next-line solid/reactivity -- run() is only called from event handlers
    run(async () => {
      const name = newName().trim();
      setCreating(null);
      setNewName("");
      if (!name) return;
      // git checks the files out, which takes a while in a big repo: show the row at once
      setPendingWt({ project: projectId, name });
      const stamp = navStamp();
      let w;
      try {
        w = await createWorktree(projectId, name);
      } catch (e) {
        // a failed create gives the dialog back with the name still typed; run() shows the error
        setNewName(name);
        setCreating(projectId);
        throw e;
      } finally {
        setPendingWt(null);
      }
      selectIfStill(stamp, w.path);
      showNotice(T.created(name, w.env_copied?.length ? T.envCopied(w.env_copied.length) : "", w.env_skipped?.length ? T.envSkipped(w.env_skipped.length) : ""));
      // the worktree exists by now: a failing agent must not skip the setup, and the user reads a sentence, not the raw error
      if (newAgent() !== "none") {
        try {
          await launchDefaultAgent(projectId, w.path, newAgent(), AGENTS[newAgent()], undefined, undefined, projectTerse(projectId));
        } catch (e) {
          showNotice(T.agentFailed(name, errorText(e).slice(0, 120)));
        }
      }
      let setup = projects().find((p) => p.id === projectId)?.setup;
      if (!setup && (await getSettings().then((x) => x.auto_install, () => false))) {
        const pm = await detectPackageManagers(projectId).catch(() => null);
        if (pm?.package_json && pm.suggested) setup = `${pm.suggested} install`;
      }
      if (setup) await runSetup(projectId, w.path, setup);
    });

  // leaving the new-worktree dialog with a name typed in asks first
  const closeNewWorktree = async () => {
    if (newName().trim() && !(await askDiscard(T.discardWtName))) return;
    setCreating(null);
    setNewName("");
  };
  const closeSetup = async (setup?: string | null) => {
    if (setupText() !== (setup ?? "") && !(await askDiscard(T.discardSetup))) return;
    setEditSetup(null);
  };
  const saveSetup = (projectId: string) =>
    // eslint-disable-next-line solid/reactivity -- run() is only called from event handlers
    run(async () => {
      await setProjectSetup(projectId, setupText());
      setEditSetup(null);
    });

  const menuDeps: MenuDeps = {
    ...actions,
    refresh: async () => {
      await refreshProjects();
      await refreshSessions();
    },
    rename: (path, current, label) => {
      setWtNewName(current);
      setWtRenameErr("");
      setRenamingWt({ path, name: label });
    },
    afterRebase: () => void pollOverlaps(),
    newWorktree: (id) => setCreating(id),
    runSetup: (projectId, path, setup) =>
      run(async () => {
        if (!(await runSetup(projectId, path, setup))) showNotice(T.noSetup);
      }),
    editSetup: (id, setup) => {
      if (isMultiLine(setup)) return setShowProjectSettings(id);
      setSetupText(setup);
      setEditSetup(id);
    },
  };

  createEffect(() => {
    const r = removeRequest();
    if (!r) return;
    setRemoveRequest(null);
    void dropWorktree(r.project, r.path, r.name);
  });

  // project filter: by project name or by the branch of one of its worktrees
  const [filter, setFilter] = createSignal("");
  const shownProjects = createMemo(() => {
    const q = filter().trim().toLowerCase();
    if (!q) return projects();
    return projects().filter((p) => p.name.toLowerCase().includes(q) || p.worktrees.some((w) => (w.branch ?? "").toLowerCase().includes(q)));
  });
  const footerButton = "justify-start gap-1.5 rounded-md px-2.5 py-2 text-sm hover:bg-foreground/5";
  return (
    <aside class="sidebar">
      <div class="brand">
        <img src={logo} alt="" />
        Axelstra
      </div>
      <Button variant="default" onClick={() => setShowAddProject(true)}>
        {T.addProject}
      </Button>

      <Show when={projects().length > 3}>
        <Input
          type="search"
          class="sidebar-filter"
          placeholder={T.filterProjects}
          aria-label={T.filterProjects}
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Escape" && filter() && (e.stopPropagation(), setFilter(""))}
        />
      </Show>
      <div class="section">{T.projects}</div>
      <div class="sidebar-list" role="group" aria-label={T.projects}>
      <For each={shownProjects()}>
        {(p) => {
          // main first, then the tree of parents and children; the finished ones are listed apart
          const lay = createMemo(() => {
            const fin = new Set(p.worktrees.filter((w) => isFinished(w, mergedWts(), runningIn(w.path))).map((w) => wtKey(w.path)));
            return layout(p.worktrees, { finished: fin, collapsed: folded(), pinned: isPinned });
          });
          const doneOpenHere = () => doneOpen().has(p.id);
          return (
            <div class="project" classList={{ collapsed: projFolded(p.id) }}>
              <ProjectHead
                project={p}
                folded={projFolded(p.id)}
                menu={() => projectMenu(menuDeps, p)}
                onFold={() => toggleFolded(projKey(p.id))}
                onNew={() => setCreating(creating() === p.id ? null : p.id)}
                onSetup={() => {
                  setSetupText(p.setup ?? "");
                  setEditSetup(editSetup() === p.id ? null : p.id);
                }}
                onDrop={() => void dropProject(p.id, p.name)}
              />
              <Show when={editSetup() === p.id}>
                <SetupEditor name={p.name} text={setupText()} setText={setSetupText} pms={pms()} onSave={() => void saveSetup(p.id)} onClose={() => void closeSetup(p.setup)} />
              </Show>
              <Show when={pendingWt()?.project === p.id}>
                <div class="item wt pending">
                  <span class="dot work" />
                  <span class="name">{T.creating(pendingWt()!.name)}</span>
                </div>
              </Show>
              <For each={lay().rows.map((r) => r.wt)}>
                {(w) => {
                  const ri = () => lay().rows.find((r) => wtKey(r.wt.path) === wtKey(w.path));
                  const kids = () => ri()?.kids ?? 0;
                  return (
                    <WorktreeRow
                      project={p}
                      wt={w}
                      depth={ri()?.depth ?? 0}
                      kids={kids()}
                      folded={kids() > 0 && folded().has(wtKey(w.path))}
                      groupLive={groupRunning(ri()?.members ?? [], runningIn)}
                      topic={topic(w.path)}
                      overlap={overlapTitle(overlapEntries(overlaps(), w.path))}
                      overlapCount={overlapCount(overlapEntries(overlaps(), w.path))}
                      agentsHidden={agentsHidden().has(norm(w.path))}
                      menu={(label) => worktreeMenu(menuDeps, p, w, label, ri()?.members)}
                      onOpen={() => void openWorktree(w.path)}
                      onToggleFold={() => toggleFolded(wtKey(w.path))}
                      onToggleAgents={() => setAgentsHiddenList([...toggleIn(agentsHidden(), norm(w.path))])}
                      onStartLead={() => void startLead(p.id, w.path, w.branch ?? w.path)}
                      onDrop={(label) => void dropWorktree(p.id, w.path, label)}
                    />
                  );
                }}
              </For>
              <Show when={lay().finished.length > 0}>
                <div class="wt-done">
                  <div class="wt-done-head">
                    <Button type="button" variant="link" size="bare" class="gap-[5px] py-[3px] text-xs" aria-expanded={doneOpenHere()} onClick={() => setDoneOpen(toggleIn(doneOpen(), p.id))}>
                      <Icon name={doneOpenHere() ? "chevDown" : "chevRight"} size={11} />
                      <span>{T.finished(lay().finished.length)}</span>
                    </Button>
                    <Button type="button" variant="chip" size="chip" class="py-px" title={T.cleanTip} onClick={() => void cleanFinished(lay().finished.map((w) => `${p.name}: ${w.branch ?? w.path}`), loadMerged)}>
                      {T.clean}
                    </Button>
                  </div>
                  <Show when={doneOpenHere()}>
                    <For each={lay().finished}>
                      {(w) => <FinishedRow wt={w} menu={(label) => worktreeMenu(menuDeps, p, w, label)} onOpen={() => void openWorktree(w.path)} onDrop={(label) => void dropWorktree(p.id, w.path, label)} />}
                    </For>
                  </Show>
                </div>
              </Show>
            </div>
          );
        }}
      </For>

      <Show when={filter().trim() && shownProjects().length === 0}>
        <div class="empty">{T.noProjectMatch}</div>
      </Show>
      </div>
      <div class="sidebar-foot">
      <Button variant="link" size="bare" class={footerButton} onClick={() => setShowAutomations(true)}>
        <Icon name="clock" size={13} /> {T.automations}
      </Button>
      <Button variant="link" size="bare" class={footerButton} onClick={() => setShowDispatcher(true)} title={T.dispatcherTip}>
        <Icon name="branch" size={13} /> {T.dispatcher}
      </Button>
      <Button variant="link" size="bare" class={footerButton} onClick={() => setShowSettings(true)}>
        <Icon name="gear" size={13} /> {S.settings}
      </Button>
      </div>
      <Show when={creating()}>
        {(pid) => (
          <CreateWorktree
            name={projects().find((x) => x.id === pid())?.name ?? ""}
            newName={newName()}
            setNewName={setNewName}
            newAgent={newAgent()}
            setNewAgent={setNewAgentSig}
            onClose={() => void closeNewWorktree()}
            onSubmit={() => void submitWorktree(pid())}
          />
        )}
      </Show>
      <NoteView />
      <Show when={renamingWt()}>
        {(r) => (
          <RenameWorktree
            current={r().name}
            value={wtNewName()}
            error={wtRenameErr()}
            onInput={(v) => { setWtNewName(v); setWtRenameErr(""); }}
            onClose={() => setRenamingWt(null)}
            onSubmit={() => void submitWtRename()}
          />
        )}
      </Show>
    </aside>
  );
}
