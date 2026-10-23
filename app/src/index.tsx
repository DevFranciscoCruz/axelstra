import { render } from "solid-js/web";
import "@fontsource-variable/inter";
import "@xterm/xterm/css/xterm.css";
import "./tailwind.css";
import { App } from "./App";
import { Gallery } from "./ui/Gallery";
import { initTheme } from "./theme";

initTheme();

// `#ui` opens the component reference page instead of the app
render(() => (location.hash === "#ui" ? <Gallery /> : <App />), document.getElementById("root")!);
