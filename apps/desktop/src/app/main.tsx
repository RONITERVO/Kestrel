import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { LocalSpeechProvider } from "../features/speech/LocalSpeechControls";
import "./styles.css";
import "./technical-theme.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LocalSpeechProvider><App /></LocalSpeechProvider>
  </StrictMode>,
);
