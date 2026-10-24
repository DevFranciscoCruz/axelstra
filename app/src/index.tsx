import { render } from "solid-js/web";
import "@fontsource-variable/inter";
import "@xterm/xterm/css/xterm.css";
import "./tailwind.css";
import { App } from "./App";
import { lazy } from "solid-js";
import { Section } from "./Boundary";
import { initTheme } from "./theme";
import { installErrorHooks } from "./errorHooks";
import { logFrontendError } from "./api";
import { errorText } from "./appError";
import { setError } from "./store";

initTheme();
// an unhandled rejection is a failure nobody reported: log it and say so on screen instead of failing silently
installErrorHooks(
  (r) => void logFrontendError(r).catch((e) => console.debug("logFrontendError", e)),
  window,
  (reason) => setError(errorText(reason)),
);

// the component reference is its own chunk, fetched only for `#ui`
const Gallery = lazy(() => import("./ui/Gallery").then((m) => ({ default: m.Gallery })));

// `#ui` opens the component reference page instead of the app
render(
  () => (
    <Section name="app" whole>
      {location.hash === "#ui" ? <Gallery /> : <App />}
    </Section>
  ),
  document.getElementById("root")!,
);
