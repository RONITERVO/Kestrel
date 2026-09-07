import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { LocalSpeechProvider } from "../features/speech/LocalSpeechControls";
import "./styles.css";
import "./technical-theme.css";

async function mount() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { setupPreview } = await import("../preview/setup");
    setupPreview(new URLSearchParams(window.location.search).get("preview") !== "empty");
    const notice = document.createElement("div");
    notice.textContent = "UI preview · sample data · changes are not saved";
    notice.setAttribute("role", "status");
    notice.style.cssText = "position:fixed;bottom:8px;left:50%;transform:translateX(-50%);z-index:10000;padding:5px 12px;border-radius:4px;background:#15251d;color:#f3e4a3;font:12px system-ui;pointer-events:none";
    document.body.append(notice);
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <LocalSpeechProvider><App /></LocalSpeechProvider>
    </StrictMode>,
  );
}

void mount();
