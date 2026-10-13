import React from "react";
import ReactDOM from "react-dom/client";
import { focusManager, QueryClientProvider } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { queryClient } from "./lib/queryClient";
import "./styles/fonts";
import "./styles/tokens.css";
import "react-diff-view/style/index.css";
import "./styles.css";

getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  focusManager.setFocused(focused);
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
