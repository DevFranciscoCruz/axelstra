import { Show, createResource, createSignal } from "solid-js";
import { projectLogo } from "../api";

export function ProjectIcon(props: { id: string; name: string; url?: string | null }) {
  const [broken, setBroken] = createSignal(false);
  // a logo file inside the repo wins over the host avatar
  const [repoLogo] = createResource(() => props.id, projectLogo);
  const src = () => repoLogo() ?? props.url;
  return (
    <Show
      when={src() && !broken()}
      fallback={<span class="picon letter">{props.name.slice(0, 1).toUpperCase()}</span>}
    >
      <img class="picon" src={src()!} alt="" onError={() => setBroken(true)} />
    </Show>
  );
}
