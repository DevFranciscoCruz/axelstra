/**
 * Subscribe to a Tauri event from a component. `listen` is imported lazily and resolves later, so the returned stop
 * function may run before the subscription exists: it is then undone as soon as it arrives instead of leaking.
 */
export function onTauriEvent<T = unknown>(name: string, cb: (payload: T) => void): () => void {
  let un: (() => void) | undefined;
  let gone = false;
  void import("@tauri-apps/api/event")
    .then(({ listen }) => listen<T>(name, (e) => cb(e.payload)))
    .then((u) => {
      if (gone) u();
      else un = u;
    })
    .catch((e) => console.debug("listen", name, e));
  return () => {
    gone = true;
    un?.();
    un = undefined;
  };
}
