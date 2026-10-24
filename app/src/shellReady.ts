/** Waits until a freshly spawned shell has printed its prompt: the screen has text and it stopped changing between two polls. */
export async function waitShellReady(
  read: () => Promise<string[]>,
  opts: { tries?: number; every?: number; sleep?: (ms: number) => Promise<void> } = {},
): Promise<boolean> {
  const tries = opts.tries ?? 40;
  const every = opts.every ?? 150;
  const sleep = opts.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  let last = "";
  for (let i = 0; i < tries; i++) {
    let text = "";
    try {
      text = (await read()).join("\n").trim();
    } catch {
      /* the session is not readable yet */
    }
    if (text && text === last) return true;
    last = text;
    await sleep(every);
  }
  return false;
}
