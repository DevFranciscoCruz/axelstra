import { render } from "solid-js/web";
import "@fontsource-variable/inter";
import "@xterm/xterm/css/xterm.css";
import "./tailwind.css";
import { lazy } from "solid-js";
import { Section } from "./Boundary";
import { initTheme } from "./theme";
import { installErrorHooks } from "./errorHooks";
import { logFrontendError } from "./api";
import { errorText } from "./appError";
import { hydrateUi } from "./uiState";
import { initLocale } from "./i18n";
import { installNativeMenuGuard } from "./nativeMenu";

initTheme();
installNativeMenuGuard();
// an unhandled rejection is a failure nobody reported: log it and say so on screen instead of failing silently
installErrorHooks(
  (r) => void logFrontendError(r).catch((e) => console.debug("logFrontendError", e)),
  window,
  // the store is imported after the UI state is restored (see below), so it is looked up when needed
  (reason) => void import("./store").then((m) => m.setError(errorText(reason))),
);

// the component reference is its own chunk, fetched only for `#ui`
const Gallery = lazy(() => import("./ui/Gallery").then((m) => ({ default: m.Gallery })));

// the signals read localStorage when their module loads, so what the daemon kept for this profile (pins, folded groups,
// notifications) is restored first and App/store are imported only afterwards
void hydrateUi()
  .catch((e) => console.debug("hydrateUi", e))
  .then(initLocale)
  .then(() => import("./App"))
  .then(({ App }) => {
    // `#ui` opens the component reference page instead of the app
    render(
      () => (
        <Section name="app" whole>
          {location.hash === "#ui" ? <Gallery /> : <App />}
        </Section>
      ),
      document.getElementById("root")!,
    );
  });
