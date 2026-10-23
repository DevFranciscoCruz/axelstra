import { render } from "solid-js/web";
import "@fontsource-variable/inter";
import "@xterm/xterm/css/xterm.css";
import "./tailwind.css";
import { App } from "./App";
import { initTheme } from "./theme";

initTheme();

render(() => <App />, document.getElementById("root")!);
