import "@testing-library/jest-dom/vitest";
import { beforeEach } from "vitest";
import { setupPreview } from "./preview/setup";

// UI tests explicitly select fixtures; production IPC has no silent browser fallback.
beforeEach(() => setupPreview(false));
