/** Open modals, oldest first. Only the last one (the top) reacts to Esc and Tab. */
const stack: symbol[] = [];

export function pushModal(): symbol {
  const id = Symbol("modal");
  stack.push(id);
  return id;
}

export function popModal(id: symbol) {
  const i = stack.indexOf(id);
  if (i >= 0) stack.splice(i, 1);
}

export const isTopModal = (id: symbol) => stack[stack.length - 1] === id;

/** Test hook. */
export const modalDepth = () => stack.length;
