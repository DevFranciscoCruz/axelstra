import { createSignal } from "solid-js";

const [forms, setForms] = createSignal<ReadonlySet<string>>(new Set());

/** a form of the dialog has typed text: clicking the backdrop must not throw it away */
export const addProjectDirty = () => forms().size > 0;
/** `form` says which tab; a tab clears its mark when it goes away */
export const markAddProjectForm = (form: string, dirty: boolean) =>
  setForms((s) => {
    if (s.has(form) === dirty) return s;
    const n = new Set(s);
    if (dirty) n.add(form);
    else n.delete(form);
    return n;
  });
