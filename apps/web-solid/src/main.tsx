import { render } from "solid-js/web";
import "@/ui/tokens.css";
import "@/ui/primitives.css";
import "@/shell/shell.css";
import "@/shell/theme";
import "@/core/chat";
import { start } from "@/core/connection";
import { App } from "@/shell/App";
import { state } from "@/core/store";
import { view } from "@/core/view";
import { startAnalytics } from "@/core/analytics";

// Read-only handle for the e2e harness; the page can already read its own state.
(window as unknown as { __zuse: unknown }).__zuse = { state, view };

render(() => <App />, document.getElementById("root")!);
startAnalytics();
void start();
