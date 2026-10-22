import { render } from "solid-js/web";
import "@fontsource-variable/inter";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { App } from "./App";

render(() => <App />, document.getElementById("root")!);
