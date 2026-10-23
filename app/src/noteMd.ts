/** Block of a task file, enough markdown for what the daemon writes: headings, bullet lists, code fences, paragraphs. */
export type MdBlock = { kind: "h"; level: number; text: string } | { kind: "li"; text: string } | { kind: "code"; text: string } | { kind: "p"; text: string };

export const parseNote = (src: string): MdBlock[] => {
  const out: MdBlock[] = [];
  let para: string[] = [];
  let fence: string[] | null = null;
  const flush = () => {
    if (para.length) out.push({ kind: "p", text: para.join("\n") });
    para = [];
  };
  for (const raw of src.replace(/\r\n?/g, "\n").split("\n")) {
    if (fence) {
      if (raw.trimStart().startsWith("```")) {
        out.push({ kind: "code", text: fence.join("\n") });
        fence = null;
      } else fence.push(raw);
      continue;
    }
    if (raw.trimStart().startsWith("```")) {
      flush();
      fence = [];
      continue;
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(raw);
    const li = /^\s*[-*]\s+(.*)$/.exec(raw);
    if (h) {
      flush();
      out.push({ kind: "h", level: h[1].length, text: h[2].trim() });
    } else if (li) {
      flush();
      out.push({ kind: "li", text: li[1] });
    } else if (!raw.trim()) flush();
    else para.push(raw);
  }
  // an unclosed fence still shows what it held
  if (fence) out.push({ kind: "code", text: fence.join("\n") });
  flush();
  return out;
};
