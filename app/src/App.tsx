import { Icon } from "./Icon";
import { Button, cn } from "./ui";
import { For, Match, Show, Switch, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { SessionInfo, defaultShell, launchDefaultAgent, killSession, removeSession, spawnSession } from "./api";
import { Section } from "./Boundary";
import { KEEP, pruneKept, touchKept } from "./keptTerms";
import { ChangesPanel } from "./ChangesPanel";
import { CommitView } from "./CommitView";
import { Editor } from "./Editor";
import { FilesPanel } from "./FilesPanel";
import { DockerView } from "./Docker";
import { Kanban } from "./Kanban";
import { DiffView } from "./DiffView";
import { Sidebar } from "./Sidebar";
import { WorktreeReports } from "./WorktreeReports";
import { Palette } from "./Palette";
import { Welcome } from "./Welcome";
import { ContextMenu } from "./ContextMenu";
import { Automations } from "./Automations";
import { Dispatcher } from "./Dispatcher";
import { Settings } from "./Settings";
import { AddProject } from "./AddProject";
import { Shortcuts } from "./Shortcuts";
import { S } from "./strings";
import { startDrag } from "./dragResize";
import { LEFT, RIGHT, resizeTo } from "./layout";
import { K } from "./keyboard";
import { buildPaletteActions } from "./paletteActions";
import { AgentPickerModal } from "./AgentPickerModal";
import { SplitPane } from "./SplitPane";
import { useBootstrap } from "./useBootstrap";
import { useGlobalShortcuts } from "./useGlobalShortcuts";
import { openUrl } from "@tauri-apps/plugin-opener";
import { UpdateToast } from "./UpdateToast";
import { VersionSkew } from "./VersionSkew";
import { PanelIcon } from "./PanelIcon";
import { CloseConfirm } from "./CloseConfirm";
import { ConfirmHost } from "./Confirm";
import { RemoveProjectHost } from "./RemoveProject";
import { AgentSearch } from "./AgentSearch";
import { Orchestra } from "./Orchestra";
import { UsageChip } from "./Usage";
import { computerEnable } from "./api";
import { ProjectSettings } from "./ProjectSettings";
import { StatusBar } from "./StatusBar";
import { MicIcon, VoiceDraft, listening, toggleVoice, voiceSupported } from "./Voice";
import { ErrorToast } from "./ErrorToast";
import { TabBar } from "./TabBar";
import { ask } from "./Confirm";
import { PullsPanel } from "./PullsPanel";
import { TimelinePanel } from "./TimelinePanel";
import { TerminalView } from "./TerminalView";
import {
  activeSession,
  AGENTS,
  agentPrompt,
  pickAgent,
  fileLine,
  projectTerse,
  changesMode,
  showAddProject,
  setShowAddProject,
  showAutomations,
  showDispatcher,
  setSessionCloser,
  showSettings,
  showAgentSearch,
  showShortcuts,
  showView,
  projectAgent,
  tabLabel,
  sortedTabs,
  rememberClosed,
  popClosed,
  showOrchestra,
  computerOn,
  setComputerOn,
  ports,
  splitSession,
  setSplitSession,
  projects,
  setCreatingWorktree,
  findWorktree,
  view,
  openFile,
  rightTab,
  setShowProjectSettings,
  showProjectSettings,
  refreshChanges,
  refreshSessions,
  sessions,
  showNotice,
  openIfStill,
  navStamp,
  selectedWorktree,
  selectWorktree,
  sessionsOf,
  setActiveSession,
  setError,
  showSidebar,
  showRightPanel,
  toggleSidebar,
  toggleRightPanel,
  guarded,
  leftW,
  rightW,
  setLeftW,
  setRightW,
} from "./store";

export function App() {
  // under 1000px the right panel is hidden by CSS, so its toggles would do nothing
  const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 1000px)") : null;
  const [narrow, setNarrow] = createSignal(narrowQuery?.matches ?? false);
  onMount(() => {
    if (!narrowQuery) return;
    const on = (e: MediaQueryListEvent) => setNarrow(e.matches);
    narrowQuery.addEventListener("change", on);
    onCleanup(() => narrowQuery.removeEventListener("change", on));
  });
  // screen control for agents is a deliberate, per-run permission: asking first, and it ends when the daemon does
  const toggleComputer = async () => {
    try {
      if (computerOn()) {
        setComputerOn(await computerEnable(false));
        return;
      }
      const ok = await ask(S.app.computerAsk, { title: S.app.computerTitle, kind: "warning", okLabel: S.app.allow, cancelLabel: S.cancel });
      if (ok) setComputerOn(await computerEnable(true));
    } catch (e) {
      setError(String(e));
    }
  };
  useBootstrap();

  const tabs = () => sortedTabs(sessionsOf(selectedWorktree()));
  // making the session shown in the split pane the active one swaps the two, so the pane never ends up empty
  const activate = (id: number) => {
    if (id === splitSession()) setSplitSession(activeSession() !== id ? activeSession() : null);
    setActiveSession(id);
  };
  const showTab = (s: SessionInfo) =>
    guarded(() => {
      showView();
      activate(s.id);
    });

  // Terminals of the last few tabs stay mounted (hidden): switching back is instant, keeps the scroll position and
  // does not replay the history. Older ones are unmounted. Order is stable so nothing is moved in the DOM.
  const [kept, setKept] = createSignal<number[]>([]);
  const lastUsed = new Map<number, number>();
  createEffect(() => {
    const id = activeSession();
    if (id === null || id === undefined) return;
    lastUsed.set(id, Date.now());
    setKept((k) => touchKept(k, id, lastUsed, KEEP));
  });
  // a closed session must not keep a slot (it would push a live terminal out) nor a lastUsed entry
  createEffect(() => {
    const alive = new Set(sessions().map((s) => s.id));
    setKept((k) => pruneKept(k, alive, lastUsed));
  });
  const keptLive = () => kept().filter((id) => (id !== splitSession() || id === activeSession()) && sessions().some((s) => s.id === id));
  const current = () => findWorktree(selectedWorktree());

  // reload the change list when the worktree, its project data or the compare mode changes
  createEffect(on([selectedWorktree, () => current()?.wt.path, changesMode], () => void refreshChanges()));

  // the same session in both panes would leave one of them blank (e.g. after a notification opened the split one)
  createEffect(() => {
    if (splitSession() !== null && splitSession() === activeSession()) setSplitSession(null);
  });

  // keep one terminal tab selected whenever the worktree has terminals
  createEffect(() => {
    const t = tabs();
    const a = activeSession();
    if (t.length > 0 && (a === null || !t.some((s) => s.id === a))) {
      // a worktree opens on its own agent first (the main worktree has its own session, apart from the tasks), else on its first terminal
      setActiveSession((t.find((s) => s.meta?.kind === "agent" && s.running) ?? t[0]).id);
    }
    if (t.length === 0 && a !== null) setActiveSession(null);
  });

  const spawnShell = async (c: NonNullable<ReturnType<typeof current>>) => {
    const [program, args] = await defaultShell(c.wt.path);
    const { id } = await spawnSession(program, args, "shell", c.wt.path, { project: c.project.id, worktree: c.wt.path, kind: "shell" });
    await refreshSessions();
    return id;
  };

  const newTerminal = async () => {
    const c = current();
    if (!c) return;
    try {
      const id = await spawnShell(c);
      showView();
      activate(id);
    } catch (e) {
      setError(String(e));
    }
  };

  // split view: show the next terminal beside the active one, opening a fresh shell if there is only one
  const toggleSplit = async () => {
    if (splitSession() !== null) {
      setSplitSession(null);
      return;
    }
    const other = tabs().find((s) => s.id !== activeSession());
    if (other) {
      setSplitSession(other.id);
      return;
    }
    const c = current();
    if (!c) return;
    try {
      setSplitSession(await spawnShell(c));
    } catch (e) {
      setError(String(e));
    }
  };

  const newAgent = async () => {
    const c = current();
    if (!c) return;
    // asks which agent to open (or a plain terminal); a choice becomes the default
    const agent = await pickAgent();
    if (!agent) return;
    if (agent === "shell") return void newTerminal();
    try {
      const id = await launchDefaultAgent(c.project.id, c.wt.path, agent, AGENTS[agent], undefined, undefined, projectTerse(c.project.id));
      await refreshSessions();
      showView();
      activate(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const close = async (s: SessionInfo) => {
    try {
      if (s.running) {
        const agent = s.meta?.kind === "agent";
        const ok = await ask(S.app.closeAsk(tabLabel(s), agent), { title: agent ? S.app.closeAgent : S.app.closeTerminal, kind: "warning", okLabel: S.close, cancelLabel: S.cancel });
        if (!ok) return;
      }
      rememberClosed(s);
      if (s.running) await killSession(s.id);
      await removeSession(s.id);
    } catch (e) {
      setError(String(e));
    }
    await refreshSessions();
  };

  setSessionCloser(close);

  // Ctrl+Shift+T: bring back the tab closed last (a shell opens anew; an agent resumes its conversation)
  const reopenClosed = async () => {
    const t = popClosed();
    if (!t) return showNotice(S.app.noClosed);
    const target = findWorktree(t.meta?.worktree ?? null) ?? current();
    if (!target) return;
    const m = t.meta;
    const stamp = navStamp();
    try {
      const id =
        m?.kind === "agent" && m.agent
          ? await launchDefaultAgent(target.project.id, target.wt.path, t.label ?? m.agent, m.agent, { key: m.key, resume: m.resume, lead: m.lead }, undefined, projectTerse(target.project.id))
          : await spawnShell(target);
      await refreshSessions();
      openIfStill(stamp, target.wt.path, id);
    } catch (e) {
      setError(String(e));
    }
  };

  useGlobalShortcuts({ tabs, showTab, reopenClosed });

  // tab being renamed (double click); the palette starts it too
  const [renaming, setRenaming] = createSignal<number | null>(null);

  // drag a panel edge; `dir` flips the delta for the right-hand panel
  let stopDrag: (() => void) | undefined;
  onCleanup(() => stopDrag?.());
  const drag = (e: MouseEvent, dir: 1 | -1, start: number, set: (w: number) => void) => {
    e.preventDefault();
    const x0 = e.clientX;
    stopDrag?.();
    stopDrag = startDrag((m) => set(start + dir * (m.clientX - x0)));
  };

  // the dialogs for a new or removed worktree belong to the sidebar: show it first
  const showSidebarNow = () => {
    if (!showSidebar()) toggleSidebar();
  };

  // the same resize from the keyboard (arrows, Shift for bigger steps, Home/End)
  const resizeKey = (e: KeyboardEvent, dir: 1 | -1, w: number, range: { min: number; max: number }, set: (w: number) => void) => {
    const to = resizeTo(w, e.key, e.shiftKey, dir, range);
    if (to === null) return;
    e.preventDefault();
    set(to);
  };

  const paletteActions = () =>
    buildPaletteActions({ current, newAgent, newTerminal, toggleSplit, reopenClosed, toggleComputer, computerOn, startRename: setRenaming, showSidebarNow });

  return (
    <>
    <div
      class="app"
      data-app-shell
      classList={{ "with-right": current() !== null && showRightPanel(), "no-left": !showSidebar() }}
      style={{ "--left-w": `${leftW()}px`, "--right-w": `${rightW()}px` }}
    >
      <Show when={showSidebar()}>
        <div
          class="resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label={S.app.leftWidth}
          aria-valuenow={leftW()}
          aria-valuemin={LEFT.min}
          aria-valuemax={LEFT.max}
          tabindex="0"
          onKeyDown={(e) => resizeKey(e, 1, leftW(), LEFT, setLeftW)}
          onMouseDown={(e) => drag(e, 1, leftW(), setLeftW)}
        />
      </Show>
      <Show when={current() && showRightPanel()}>
        <div
          class="resizer right-edge"
          role="separator"
          aria-orientation="vertical"
          aria-label={S.app.rightWidth}
          aria-valuenow={rightW()}
          aria-valuemin={RIGHT.min}
          aria-valuemax={RIGHT.max}
          tabindex="0"
          onKeyDown={(e) => resizeKey(e, -1, rightW(), RIGHT, setRightW)}
          onMouseDown={(e) => drag(e, -1, rightW(), setRightW)}
        />
      </Show>
      <Show when={agentPrompt()}>{(done) => <AgentPickerModal project={current()?.project.id} done={done()} />}</Show>
      <Palette actions={paletteActions} />
      <Show when={showShortcuts()}>
        <Shortcuts />
      </Show>
      <Show when={showAgentSearch()}>
        <AgentSearch />
      </Show>
      <Show when={showOrchestra()}>
        <Orchestra />
      </Show>
      <Show when={showSettings()}>
        <Settings />
      </Show>
      <Show when={showProjectSettings()} keyed>
        {(id) => <ProjectSettings projectId={id} onClose={() => setShowProjectSettings(null)} />}
      </Show>
      <Show when={showDispatcher()}>
        <Dispatcher />
      </Show>
      <Show when={showAddProject()}>
        <AddProject />
      </Show>
      <Show when={showAutomations()}>
        <Automations />
      </Show>
      <CloseConfirm />
      <ConfirmHost />
      <RemoveProjectHost />
      <Show when={showSidebar()}>
        <Section name={S.app.sectionProjects}>
          <Sidebar />
        </Section>
      </Show>
      <main class="main">
        <Show
          when={current()}
          fallback={
            <>
              <Show when={!showSidebar()}>
                <Button variant="icon" size="tool" class="m-1" title={K(S.app.showProjects)} aria-label={K(S.app.showProjects)} onClick={toggleSidebar}>
                  <PanelIcon side="left" on={false} />
                </Button>
              </Show>
              <Show when={projects()[0]} fallback={<Welcome onAddProject={() => setShowAddProject(true)} />}>
                {(p) => (
                  <Welcome
                    onAddProject={() => setShowAddProject(true)}
                    projectName={p().name}
                    worktrees={p().worktrees}
                    onPick={selectWorktree}
                    onNewWorktree={() => {
                      showSidebarNow();
                      setCreatingWorktree(p().id);
                    }}
                  />
                )}
              </Show>
            </>
          }
        >
          <div class="tabbar">
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", showSidebar() && "text-foreground")} title={K(S.app.projects)} aria-label={K(S.app.projects)} onClick={toggleSidebar}>
              <PanelIcon side="left" on={showSidebar()} />
            </Button>
            <TabBar tabs={tabs()} renaming={renaming()} setRenaming={setRenaming} showTab={showTab} close={close} />
            <span class="grow" />
            <UsageChip />
            <Show when={computerOn()}>
              <span class="computer-on" title={S.app.screenOnTip}><i class="bdot" /> {S.app.screen}</span>
            </Show>
            <For each={ports()}>
              {(port) => (
                <Button variant="success" size="chip" class="ml-1.5 self-center rounded-xl" title={S.app.openPort(port)} onClick={() => void openUrl(`http://localhost:${port}`)}>
                  :{port} <Icon name="external" size={11} inline />
                </Button>
              )}
            </For>
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", listening() && "text-destructive")} title={K(S.app.voice)} aria-label={K(S.app.voice)} onClick={() => void toggleVoice()} hidden={!voiceSupported}>
              <MicIcon />
            </Button>
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", splitSession() !== null && "text-foreground")} title={S.app.split} aria-label={S.app.split} onClick={() => void toggleSplit()}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M5.5 3v10M10.5 3v10" />
              </svg>
            </Button>
            <Button variant="icon" size="tool" class="ml-1 self-center" title={S.app.newAgentOf(projectAgent(current()?.project.id))} aria-label={S.app.newAgentOf(projectAgent(current()?.project.id))} onClick={newAgent}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                <path d="M8 1.5c.4 3.4 1.6 4.6 5 5-3.4.4-4.6 1.6-5 5-.4-3.4-1.6-4.6-5-5 3.4-.4 4.6-1.6 5-5z" />
              </svg>
            </Button>
            <Button variant="icon" size="tool" class="ml-1 self-center" title={S.app.newTerminal} aria-label={S.app.newTerminal} onClick={newTerminal}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M8 3.5v9M3.5 8h9" />
              </svg>
            </Button>
            <Show when={!showRightPanel() && !narrow()}>
              <Button variant="icon" size="tool" class="ml-1 self-center" title={K(S.app.showRight)} aria-label={K(S.app.showRight)} onClick={toggleRightPanel}>
                <PanelIcon side="right" on={false} />
              </Button>
            </Show>
          </div>
          <VoiceDraft target={activeSession()} />
          <div class="term-area">
            <Section name={S.app.sectionTerminals}>
            <Switch
              fallback={
              <Show
                when={activeSession() !== null}
                fallback={
                  <div class="placeholder empty">
                    <div>{S.app.noTerminals}</div>
                    <Show when={current()}>{(c) => <WorktreeReports project={c().project.id} worktree={c().wt.path} />}</Show>
                    <div class="empty-actions">
                      <Button variant="default" onClick={() => void newAgent()}>
                        <Icon name="sparkle" size={12} inline /> {S.app.openAgent}
                      </Button>
                      <Button onClick={() => void newTerminal()}>+ Terminal</Button>
                    </div>
                    <div class="hints">
                      <kbd>{K("Ctrl+K")}</kbd> {S.app.hintPalette} · <kbd>{K("?")}</kbd> {S.app.hintShortcuts} · <kbd>{K("Ctrl+B")}</kbd> {S.app.hintProjects} · <kbd>{K("Ctrl+Shift+B")}</kbd> {S.app.hintRight}
                    </div>
                  </div>
                }
              >
                <For each={keptLive()}>
                  {(id) => (
                    <div class="term-slot" classList={{ off: id !== activeSession() }}>
                      <TerminalView id={id} active={() => id === activeSession()} onExit={() => void refreshSessions()} />
                    </div>
                  )}
                </For>
                <Show when={splitSession() !== null && splitSession() !== activeSession() && tabs().some((s) => s.id === splitSession())}>
                  <SplitPane />
                </Show>
              </Show>
              }
            >
              <Match when={view().kind === "docker"}>
                <DockerView />
              </Match>
              <Match when={view().kind === "kanban"}>
                <Kanban project={current()!.project.id} />
              </Match>
              <Match when={view().kind === "commit"}>
                <CommitView />
              </Match>
              <Match when={openFile()} keyed>
                {(f) => <Editor wt={selectedWorktree()!} path={f} initialLine={fileLine() ?? undefined} />}
              </Match>
              <Match when={view().kind === "diff"}>
                <DiffView />
              </Match>
            </Switch>
            </Section>
          </div>
        </Show>
      </main>
      <Show when={current() && showRightPanel()}>
        <Section name={S.app.sectionRight}>
          <Show when={rightTab() === "files"} fallback={rightTab() === "pulls" ? <PullsPanel /> : rightTab() === "timeline" ? <TimelinePanel /> : <ChangesPanel />}>
            <FilesPanel />
          </Show>
        </Section>
      </Show>
      <StatusBar />
    </div>
    {/* outside the shell: menus and toasts stay usable while a dialog makes the shell inert */}
    <ContextMenu />
    <ErrorToast>
      <UpdateToast />
      <VersionSkew />
    </ErrorToast>
    </>
  );
}
