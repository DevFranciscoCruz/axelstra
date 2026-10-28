import { L } from "./strings.logic";

/** what the user reads when the microphone or the transcription fails: a sentence and a way out, not the raw exception */
export const micErrorText = (e: unknown): string => {
  const name = (e as { name?: string } | null)?.name ?? "";
  const raw = String(e);
  if (name === "NotAllowedError" || /NotAllowed|Permission denied/i.test(raw)) return L.voice.denied;
  if (name === "NotFoundError" || /NotFound|Requested device not found/i.test(raw)) return L.voice.missing;
  if (name === "NotReadableError" || /NotReadable|in use/i.test(raw)) return L.voice.busy;
  return L.voice.failed;
};

export const transcribeErrorText = (e: unknown): string => {
  const raw = String(e)
    .replace(/^Error: /, "")
    .slice(0, 120);
  return /model/i.test(raw) ? L.voice.noModel : L.voice.transcribeFailed;
};

export const downloadErrorText = (e: unknown): string => {
  const raw = String(e);
  if (/does not match the expected hash/i.test(raw)) return L.voice.downloadCorrupt;
  if (/no space|disk full|os error (28|112)|access is denied|permission denied|read-only/i.test(raw))
    return L.voice.downloadDisk;
  return L.voice.downloadFailed;
};
