/**
 * Follow the pointer while a drag handle is held. The drag ends on mouseup, when the window loses focus, or when a move
 * arrives with no button down (the mouseup happened outside the window): the listeners never stay behind.
 * Returns the function that ends it early (call it from onCleanup).
 */
export function startDrag(onMove: (e: MouseEvent) => void): () => void {
  let active = true;
  const end = () => {
    if (!active) return;
    active = false;
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", end);
    window.removeEventListener("blur", end);
    document.body.classList.remove("resizing");
  };
  const move = (e: MouseEvent) => {
    if (e.buttons === 0) return end();
    onMove(e);
  };
  document.body.classList.add("resizing");
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", end);
  window.addEventListener("blur", end);
  return end;
}
