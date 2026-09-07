import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DesktopCommands, DesktopEvents } from "../contracts/index";

export type PreviewHandlers = Partial<{
  [C in keyof DesktopCommands]: (args: DesktopCommands[C]["args"]) => DesktopCommands[C]["result"] | Promise<DesktopCommands[C]["result"]>;
}>;
let previewHandlers: PreviewHandlers | undefined;

/** Called only by the development entry point. A packaged desktop cannot install mocks. */
export function installPreviewHandlers(handlers: PreviewHandlers): void {
  if (!import.meta.env.DEV || "__TAURI_INTERNALS__" in window) {
    throw new Error("Preview handlers are only available in the development browser.");
  }
  previewHandlers = handlers;
}

/** The only native transport. Names, arguments, results and payloads all come from Rust. */
export function invoke<C extends keyof DesktopCommands>(
  command: C,
  ...args: DesktopCommands[C]["args"] extends Record<string, never>
    ? [args?: DesktopCommands[C]["args"]]
    : [args: DesktopCommands[C]["args"]]
): Promise<DesktopCommands[C]["result"]> {
  if (previewHandlers) {
    const handler = previewHandlers[command];
    if (!handler) return Promise.reject(new Error("This action needs the desktop application. The browser preview uses sample data."));
    // TypeScript cannot correlate an indexed generic function with its matching mapped argument.
    // Both are checked against the same Rust-generated command key above.
    return Promise.resolve(handler(args[0] as DesktopCommands[C]["args"]));
  }
  return tauriInvoke(command, args[0]);
}

export function listen<E extends keyof DesktopEvents>(
  event: E,
  handler: (event: { payload: DesktopEvents[E] }) => void,
): Promise<UnlistenFn> {
  if (previewHandlers) return Promise.resolve(() => undefined);
  return tauriListen<DesktopEvents[E]>(event, handler);
}

export type { UnlistenFn };
