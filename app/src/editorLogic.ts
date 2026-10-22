/** Above this many characters the editor asks before opening (CodeMirror slows down on big files). */
export const LARGE_FILE_CHARS = 2 * 1024 * 1024;

/** Text for a failed fs.read: binary and non-UTF-8 files cannot be edited, say so clearly. */
export function describeReadError(err: unknown): string {
  const s = String(err);
  if (/binary file/i.test(s)) return "Ficheiro binário: não pode ser aberto no editor.";
  if (/not valid UTF-8/i.test(s)) return "O ficheiro não é texto UTF-8: não pode ser aberto no editor.";
  if (/larger than/i.test(s)) return "Ficheiro demasiado grande para o editor.";
  return s;
}

/** True when a file of this size should be confirmed by the user before opening. */
export const isLarge = (chars: number) => chars > LARGE_FILE_CHARS;

/** Size as shown in the large-file warning, e.g. "3,4 MB". */
export const sizeLabel = (chars: number) => `${(chars / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;

/**
 * Cheap dirty check run on every keystroke: lengths differ -> dirty without building the string;
 * the full toString() compare only happens when the lengths match.
 */
export function docDiffers(docLength: number, base: string, text: () => string): boolean {
  if (docLength !== base.length) return true;
  return text() !== base;
}

/** 1-based line number clamped into the document, or null when there is nothing to jump to. */
export function clampLine(line: number | undefined, total: number): number | null {
  if (line === undefined || !Number.isFinite(line) || total < 1) return null;
  return Math.min(Math.max(Math.trunc(line), 1), total);
}
