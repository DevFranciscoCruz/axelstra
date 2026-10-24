/** what the user reads when the microphone or the transcription fails: a sentence and a way out, not the raw exception */
export const micErrorText = (e: unknown): string => {
  const name = (e as { name?: string } | null)?.name ?? "";
  const raw = String(e);
  if (name === "NotAllowedError" || /NotAllowed|Permission denied/i.test(raw)) return "O acesso ao microfone foi recusado. Permite-o nas definições de privacidade do Windows.";
  if (name === "NotFoundError" || /NotFound|Requested device not found/i.test(raw)) return "Não foi encontrado nenhum microfone. Liga um e tenta de novo.";
  if (name === "NotReadableError" || /NotReadable|in use/i.test(raw)) return "O microfone está a ser usado por outra aplicação.";
  return "Não foi possível usar o microfone. Tenta de novo.";
};

export const transcribeErrorText = (e: unknown): string => {
  const raw = String(e).replace(/^Error: /, "").slice(0, 120);
  return /model/i.test(raw) ? "O modelo de ditado não está disponível. Descarrega-o de novo." : "A transcrição falhou. Tenta ditar outra vez.";
};

export const downloadErrorText = (_e: unknown): string => "Não foi possível descarregar o modelo de ditado. Verifica a ligação e tenta de novo.";
