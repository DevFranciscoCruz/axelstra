import { Compartment, EditorState, Extension } from "@codemirror/state";
import { Icon } from "./Icon";
import { EditorView, drawSelection, highlightActiveLine, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput, syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { searchKeymap } from "@codemirror/search";
import { oneDark } from "@codemirror/theme-one-dark";
import { Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { Opened, fsRead, fsStat, fsWrite } from "./api";
import { fsTick, guarded, setEditorDirty, setOpenFile } from "./store";
import { clampLine, describeReadError, docDiffers, isLarge, sizeLabel } from "./editorLogic";

/** Language packs load on demand so the editor itself stays small. */
async function languageFor(path: string): Promise<Extension> {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  switch (ext) {
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
    case "mjs":
    case "cjs": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ typescript: ext.startsWith("t"), jsx: ext.endsWith("x") });
    }
    case "rs":
      return (await import("@codemirror/lang-rust")).rust();
    case "json":
      return (await import("@codemirror/lang-json")).json();
    case "md":
      return (await import("@codemirror/lang-markdown")).markdown();
    case "css":
      return (await import("@codemirror/lang-css")).css();
    case "html":
      return (await import("@codemirror/lang-html")).html();
    case "py":
      return (await import("@codemirror/lang-python")).python();
    default:
      return [];
  }
}

/** `initialLine` (1-based) scrolls to and selects that line once the file is loaded (terminal links like file:12). */
export function Editor(props: { wt: string; path: string; initialLine?: number }) {
  // set once the editor is mounted: re-checks the file on disk
  let checkNow: (() => void) | undefined;
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  let meta: Opened | undefined;
  let base = ""; // text as last read/saved: dirty = doc differs from this
  const [dirty, setDirty] = createSignal(false);
  const [msg, setMsg] = createSignal("");
  const [conflict, setConflict] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [refused, setRefused] = createSignal(false); // unreadable (binary, not UTF-8, too big): nothing to edit
  const [bigSize, setBigSize] = createSignal(0); // chars of a large file waiting for the user's confirmation
  const lang = new Compartment();
  let pending: Opened | undefined; // large file read but not yet shown
  let allowBig = false;

  const gotoLine = () => {
    if (!view) return;
    const n = clampLine(props.initialLine, view.state.doc.lines);
    if (n === null) return;
    const l = view.state.doc.line(n);
    view.dispatch({ selection: { anchor: l.from, head: l.to }, effects: EditorView.scrollIntoView(l.from, { y: "center" }) });
  };

  const show = (o: Opened) => {
    pending = undefined;
    setBigSize(0);
    meta = o;
    base = o.text;
    setConflict(false);
    setMsg("");
    view?.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: o.text } });
    setDirty(false);
  };

  const load = async (first = false) => {
    let o: Opened;
    try {
      o = await fsRead(props.wt, props.path);
    } catch (e) {
      if (first) setRefused(true);
      throw new Error(describeReadError(e));
    }
    if (isLarge(o.text.length) && !allowBig) {
      pending = o;
      setBigSize(o.text.length);
      return;
    }
    show(o);
    if (first) gotoLine();
  };

  const openBig = () => {
    allowBig = true;
    if (pending) {
      show(pending);
      gotoLine();
    }
  };

  const save = async (force = false) => {
    if (!view || !meta || saving()) return;
    setSaving(true);
    try {
      const text = view.state.doc.toString();
      const r = await fsWrite(props.wt, props.path, text, meta.eol, meta.bom, force ? undefined : meta.hash);
      if (r.conflict) {
        setConflict(true);
        return;
      }
      meta = { ...meta, hash: r.hash! };
      base = text;
      setDirty(false);
      setConflict(false);
      setMsg("Guardado");
      setTimeout(() => setMsg(""), 1500);
    } catch (e) {
      setMsg(String(e));
    } finally {
      setSaving(false);
    }
  };

  let timer: ReturnType<typeof setInterval> | undefined;
  // cleanup is registered synchronously (before any await), otherwise it would never run
  onCleanup(() => clearInterval(timer));
  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: "",
        extensions: [
          lineNumbers(),
          history(),
          drawSelection(),
          highlightActiveLine(),
          indentOnInput(),
          bracketMatching(),
          syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
          oneDark,
          lang.of([]),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => (void save(), true) },
            indentWithTab,
            ...searchKeymap,
            ...historyKeymap,
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged && meta) setDirty(docDiffers(u.state.doc.length, base, () => u.state.doc.toString()));
          }),
        ],
      }),
    });
    void (async () => {
      try {
        await load(true);
        const ext = await languageFor(props.path);
        view?.dispatch({ effects: lang.reconfigure(ext) });
      } catch (e) {
        setMsg(e instanceof Error ? e.message : String(e));
      }
    })();
    // agents write to the same files: notice changes made behind our back. The daemon reports file
    // changes (fsTick); the slow timer only covers files the watcher ignores.
    const check = async () => {
      if (document.hidden || !meta || saving()) return;
      try {
        const h = await fsStat(props.wt, props.path);
        if (h === null || h === meta.hash) return;
        if (dirty()) setConflict(true);
        else await load();
      } catch {
        /* transient; the next change or tick retries */
      }
    };
    checkNow = () => void check();
    timer = setInterval(check, 30000);
  });
  // same file, new line (clicking another link to it)
  createEffect(on(() => props.initialLine, () => meta && gotoLine(), { defer: true }));
  createEffect(on(fsTick, () => checkNow?.(), { defer: true }));
  createEffect(() => setEditorDirty(dirty()));
  onCleanup(() => {
    setEditorDirty(false);
    view?.destroy();
  });

  return (
    <div class="editor">
      <div class="editor-bar">
        <span class="epath">{props.path}</span>
        <Show when={dirty()}>
          <span class="edirty" title="Alterações por guardar">
            <i class="bdot" />
          </span>
        </Show>
        <span class="emsg">{msg()}</span>
        <button class="icon" disabled={!dirty()} title="Guardar (Ctrl+S)" onClick={() => void save()}>
          Guardar
        </button>
        <button class="icon" title="Fechar" onClick={() => guarded(() => setOpenFile(null))}>
          <Icon name="x" />
        </button>
      </div>
      <Show when={conflict()}>
        <div class="econflict">
          O ficheiro mudou no disco.
          <button onClick={() => void load().catch((e) => setMsg(String(e.message ?? e)))}>Recarregar (perde as minhas alterações)</button>
          <button onClick={() => void save(true)}>Sobrescrever</button>
        </div>
      </Show>
      <Show when={bigSize() > 0}>
        <div class="econflict">
          Ficheiro grande ({sizeLabel(bigSize())}): abrir pode deixar o editor lento.
          <button onClick={openBig}>Abrir mesmo assim</button>
          <button onClick={() => guarded(() => setOpenFile(null))}>Fechar</button>
        </div>
      </Show>
      <div class="editor-host" ref={host} style={refused() || bigSize() > 0 ? { display: "none" } : undefined} />
    </div>
  );
}
