import React from "react";
import ReactDOM from "react-dom/client";
// Initializes i18next synchronously (module side effect) — must be
// imported before any component module renders a translation.
import "./lib/i18n";
import App from "./App";
import { ErrorBoundary } from "./ErrorBoundary";
import "./styles/tokens.css";
import "./App.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
