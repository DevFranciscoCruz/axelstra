import { Compartment, EditorState, Extension } from "@codemirror/state";
import { EditorView, drawSelection, highlightActiveLine, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput, syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { searchKeymap } from "@codemirror/search";
import { oneDark } from "@codemirror/theme-one-dark";
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Opened, fsRead, fsStat, fsWrite } from "./api";
import { setOpenFile } from "./store";

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

export function Editor(props: { wt: string; path: string }) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  let meta: Opened | undefined;
  let base = ""; // text as last read/saved: dirty = doc differs from this
  const [dirty, setDirty] = createSignal(false);
  const [msg, setMsg] = createSignal("");
  const [conflict, setConflict] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const lang = new Compartment();

  const load = async () => {
    const o = await fsRead(props.wt, props.path);
    meta = o;
    base = o.text;
    setConflict(false);
    setMsg("");
    view?.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: o.text } });
    setDirty(false);
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

  onMount(async () => {
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
            if (u.docChanged && meta) setDirty(u.state.doc.toString() !== base);
          }),
        ],
      }),
    });
    try {
      await load();
      view.dispatch({ effects: lang.reconfigure(await languageFor(props.path)) });
    } catch (e) {
      setMsg(String(e));
    }
    // agents write to the same files: notice changes made behind our back
    const timer = setInterval(async () => {
      if (document.hidden || !meta || saving()) return;
      try {
        const h = await fsStat(props.wt, props.path);
        if (h === null || h === meta.hash) return;
        if (dirty()) setConflict(true);
        else await load();
      } catch {
        /* transient; next tick retries */
      }
    }, 2500);
    onCleanup(() => clearInterval(timer));
  });
  onCleanup(() => view?.destroy());

  return (
    <div class="editor">
      <div class="editor-bar">
        <span class="epath">{props.path}</span>
        <Show when={dirty()}>
          <span class="edirty" title="Alterações por guardar">
            ●
          </span>
        </Show>
        <span class="emsg">{msg()}</span>
        <button class="icon" disabled={!dirty()} title="Guardar (Ctrl+S)" onClick={() => void save()}>
          Guardar
        </button>
        <button class="icon" title="Fechar" onClick={() => setOpenFile(null)}>
          ×
        </button>
      </div>
      <Show when={conflict()}>
        <div class="econflict">
          O ficheiro mudou no disco.
          <button onClick={() => void load()}>Recarregar (perde as minhas alterações)</button>
          <button onClick={() => void save(true)}>Sobrescrever</button>
        </div>
      </Show>
      <div class="editor-host" ref={host} />
    </div>
  );
}
