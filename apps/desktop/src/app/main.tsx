import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./book/tokens.css";
import "./book/base.css";
import "./book/book.css";
import "../shared/book/paging.css";
import App from "./App";
import { LocalSpeechProvider } from "../features/speech/LocalSpeechControls";

async function mount() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { setupPreview } = await import("../preview/setup");
    setupPreview(new URLSearchParams(window.location.search).get("preview") !== "empty");
    const notice = document.createElement("div");
    notice.textContent = "UI preview · sample data · changes are not saved";
    notice.setAttribute("role", "status");
    notice.className = "preview-notice";
    document.body.append(notice);
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <LocalSpeechProvider><App /></LocalSpeechProvider>
    </StrictMode>,
  );
}

void mount();
