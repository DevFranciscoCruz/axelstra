import { K } from "./keyboard";
import { Button, CheckboxInput, Textarea } from "./ui";
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Icon } from "./Icon";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeSession } from "./api";

// Local dictation: the webview records 16 kHz mono audio, the Rust side transcribes it with Whisper
// (sherpa-onnx, model downloaded on first use). Hands-free: recording stops by itself after a pause in
// the speech and the text goes straight to the terminal that was active when recording began. With
// auto-send off (or no terminal), the text lands in an editable draft and waits for confirmation.
export const voiceSupported = !!navigator.mediaDevices?.getUserMedia;
export const [voiceState, setVoiceState] = createSignal<"idle" | "recording" | "transcribing">("idle");
export const listening = () => voiceState() === "recording";
export const [draft, setDraft] = createSignal<string | null>(null);
export const [voiceError, setVoiceError] = createSignal("");
/** null: model is installed; otherwise download progress in [0,1] (0 before it starts) */
export const [needModel, setNeedModel] = createSignal(false);
export const [dlProgress, setDlProgress] = createSignal<number | null>(null);

const AUTO_KEY = "axelstra.voice.autosend";
const readAuto = () => {
  try {
    return localStorage.getItem(AUTO_KEY) !== "0";
  } catch {
    return true;
  }
};
export const [autoSend, setAutoSendSig] = createSignal(readAuto());
export const setAutoSend = (on: boolean) => {
  setAutoSendSig(on);
  try {
    localStorage.setItem(AUTO_KEY, on ? "1" : "0");
  } catch {
    /* preference only */
  }
};

/** terminal the text goes to; kept current by VoiceDraft, captured when recording starts */
let currentTarget: number | null = null;
let recTarget: number | null = null;

const SILENCE_MS = 1400;
const NO_SPEECH_MS = 8000;
const MAX_MS = 90000;

let stream: MediaStream | null = null;
let ctx: AudioContext | null = null;
let node: ScriptProcessorNode | null = null;
let chunks: Float32Array[] = [];
let lastPeak = 0;

const lang = () => (navigator.language?.toLowerCase().startsWith("pt") ? "pt" : "auto");

function release() {
  node?.disconnect();
  stream?.getTracks().forEach((t) => t.stop());
  void ctx?.close();
  node = null;
  stream = null;
  ctx = null;
}

async function transcribe() {
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const pcm = new Int16Array(total);
  let o = 0;
  for (const c of chunks) {
    for (let i = 0; i < c.length; i++) pcm[o++] = Math.max(-1, Math.min(1, c[i])) * 0x7fff;
  }
  chunks = [];
  if (total < 8000) {
    setVoiceError("Áudio demasiado curto.");
    return;
  }
  try {
    const text = await invoke<string>("voice_transcribe", pcm.buffer, { headers: { "x-lang": lang() } });
    if (!text) setVoiceError(`Nenhuma fala detetada (nível do microfone ${(lastPeak * 100).toFixed(1)}%).`);
    else if (autoSend() && recTarget !== null) {
      await writeSession(recTarget, text + "\r");
      setDraft(null);
    } else setDraft((d) => (d && d.trim() ? d.trimEnd() + " " : "") + text);
  } catch (e) {
    setVoiceError(String(e));
  }
}

export async function stopVoice() {
  if (voiceState() !== "recording") return;
  release();
  setVoiceState("transcribing");
  await transcribe();
  setVoiceState("idle");
}

/** drops the recording without transcribing it */
export function cancelVoice() {
  if (voiceState() === "recording") release();
  chunks = [];
  setVoiceState("idle");
  setDraft((d) => (d && d.trim() ? d : null));
}

export async function downloadModel() {
  setDlProgress(0);
  setVoiceError("");
  const un = await listen<{ done: number; total: number }>("voice-progress", (e) => setDlProgress(e.payload.total ? e.payload.done / e.payload.total : 0));
  try {
    await invoke("voice_download");
    setNeedModel(false);
  } catch (e) {
    setVoiceError(`Falha a descarregar o modelo: ${e}`);
  } finally {
    un();
    setDlProgress(null);
  }
}

export async function toggleVoice() {
  if (voiceState() === "recording") return void stopVoice();
  if (voiceState() === "transcribing") return;
  setVoiceError("");
  if (draft() === null) setDraft("");
  if (!(await invoke<boolean>("voice_status"))) {
    setNeedModel(true);
    return;
  }
  setNeedModel(false);
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    ctx = new AudioContext({ sampleRate: 16000 });
    const src = ctx.createMediaStreamSource(stream);
    node = ctx.createScriptProcessor(4096, 1, 1);
    // voice activity: stop after a pause that follows speech; give up if nobody speaks
    let floor = Infinity;
    let heard = false;
    let quietMs = 0;
    let elapsed = 0;
    let peak = 0;
    lastPeak = 0;
    node.onaudioprocess = (e) => {
      const data = e.inputBuffer.getChannelData(0);
      chunks.push(new Float32Array(data));
      let sum = 0;
      for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
      const rms = Math.sqrt(sum / data.length);
      const ms = (data.length / 16000) * 1000;
      elapsed += ms;
      peak = Math.max(peak, rms);
      lastPeak = peak;
      // the noise floor is the quietest recent block, creeping up slowly so a noisy room still ends the take
      floor = floor === Infinity ? rms : Math.min(rms, floor * 1.05);
      if (rms > Math.max(0.008, floor * 3)) {
        heard = true;
        quietMs = 0;
      } else quietMs += ms;
      if (heard && quietMs >= SILENCE_MS) void stopVoice();
      else if (!heard && elapsed >= NO_SPEECH_MS) {
        // nothing crossed the threshold: a dead microphone is dropped, anything else still goes to Whisper
        if (peak < 0.002) {
          cancelVoice();
          setVoiceError("Sem sinal do microfone. Verifica o microfone selecionado no Windows.");
        } else void stopVoice();
      } else if (elapsed >= MAX_MS) void stopVoice();
    };
    const mute = ctx.createGain();
    mute.gain.value = 0;
    src.connect(node);
    node.connect(mute);
    mute.connect(ctx.destination);
    chunks = [];
    recTarget = currentTarget;
    setVoiceState("recording");
  } catch (e) {
    release();
    setVoiceError(`Microfone indisponível: ${e}`);
  }
}

export const MicIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
    <rect x="6" y="1.8" width="4" height="7.4" rx="2" />
    <path d="M3.5 7.6a4.5 4.5 0 0 0 9 0M8 12.2v2" />
  </svg>
);

/** editable transcript above the terminal; Enter sends it with a newline, Esc discards */
export const VoiceDraft = (props: { target: number | null }) => {
  let ta: HTMLTextAreaElement | undefined;
  createEffect(() => {
    currentTarget = props.target;
  });
  createEffect(() => {
    if (voiceState() !== "recording") return;
    const esc = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      cancelVoice();
    };
    window.addEventListener("keydown", esc, true);
    onCleanup(() => window.removeEventListener("keydown", esc, true));
  });
  // auto-send shows only the status; the editable draft appears when it is off or the text needs a look
  const showText = () => !autoSend() || recTarget === null || !!draft()?.trim();
  const close = () => {
    release();
    chunks = [];
    setVoiceState("idle");
    setNeedModel(false);
    setDraft(null);
    setVoiceError("");
  };
  const send = async (enter: boolean) => {
    const text = draft()?.trim();
    if (!text || props.target === null) return;
    await writeSession(props.target, text + (enter ? "\r" : ""));
    close();
  };
  return (
    <Show when={draft() !== null || voiceError()}>
      <div class="voice-bar">
        <span class="voice-dot" classList={{ on: listening(), busy: voiceState() === "transcribing" }} title={listening() ? "A gravar…" : voiceState() === "transcribing" ? "A transcrever…" : "Parado"} />
        <Show when={draft() !== null && !showText()}>
          <span class="voice-status">{listening() ? "A ouvir… fale; envia ao parar de falar (Esc cancela)" : voiceState() === "transcribing" ? "A transcrever…" : ""}</span>
        </Show>
        <Show when={draft() !== null && showText()}>
          <Textarea
            ref={ta}
            class="voice-text"
            rows={Math.min(5, Math.max(1, (draft() ?? "").split("\n").length))}
            placeholder={listening() ? "A gravar… clique em Parar" : voiceState() === "transcribing" ? "A transcrever…" : "Fale ou escreva; Enter envia"}
            value={draft() ?? ""}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send(true);
              } else if (e.key === "Escape") close();
            }}
          />
          <Button size="sm" onClick={() => void toggleVoice()} disabled={voiceState() === "transcribing"} title={K("Ditado (Ctrl+Shift+M)")}>
            {listening() ? "Parar" : voiceState() === "transcribing" ? "…" : "Ditar"}
          </Button>
          <Button size="sm" onClick={() => void send(false)} title="Colar no terminal sem Enter">
            Colar
          </Button>
          <Button variant="default" size="sm" onClick={() => void send(true)}>
            Enviar
          </Button>
        </Show>
        <label class="voice-auto" title="Enviar para o terminal assim que parar de falar">
          <CheckboxInput checked={autoSend()} onChange={(e) => setAutoSend(e.currentTarget.checked)} /> Enviar automático
        </label>
        <Show when={needModel()}>
          <span class="voice-err">
            Ditado local precisa de um modelo (~160 MB, descarregado uma vez).{" "}
            <Button variant="default" size="sm" disabled={dlProgress() !== null} onClick={() => void downloadModel()}>
              {dlProgress() === null ? "Descarregar" : `${Math.round(dlProgress()! * 100)}%`}
            </Button>
          </span>
        </Show>
        <Show when={voiceError()}>
          <span class="voice-err">{voiceError()}</span>
        </Show>
        <Button variant="ghost" size="icon" class="icon" title="Fechar (Esc)" aria-label="Fechar (Esc)" onClick={close}>
          <Icon name="x" />
        </Button>
      </div>
    </Show>
  );
};
