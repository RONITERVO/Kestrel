import { test } from "node:test";
import assert from "node:assert/strict";
import { checkDesktopFile, checkNativeFile } from "./architecture-rules.mjs";

test("rejects contract duplication, including an empty replacement quarantine", () => {
  assert.ok(checkDesktopFile("contracts/index.ts", "export interface NewSettings { enabled: boolean }").length);
  assert.ok(checkDesktopFile("features/movie.ts", "interface MovieProject { id: string }", new Set(["MovieProject"])).length);
  assert.equal(checkDesktopFile("contracts/index.ts", 'export type * from "@kestrel/generated-bindings";').length, 0);
});
test("checks static, re-exported and dynamic imports against layer rules", () => {
  for (const source of ['import x from "../app/demo";', 'export * from "../app/demo";', 'const x = import("../app/demo");']) assert.ok(checkDesktopFile("platform/api.ts", source).length);
  assert.ok(checkDesktopFile("shared/button.ts", 'import { invoke } from "../platform/transport";').length);
  assert.ok(checkDesktopFile("features/editor.ts", 'import { invoke as native } from "@tauri-apps/api/core";').length);
  assert.ok(checkDesktopFile("features/editor.ts", 'import "@kestrel/generated-bindings";').length);
  assert.equal(checkDesktopFile("features/editor.ts", 'import type { MovieEdit } from "../contracts/index";').length, 0);
});
test("blocks browser persistence and direct network authority, permits the read-only legacy bridge", () => {
  for (const source of ['window.localStorage.setItem("state", "new");', 'window["indexedDB"].open("data");', 'const f = fetch; f("http://localhost");', 'new WebSocket(url);']) assert.ok(checkDesktopFile("features/editor.ts", source).length);
  assert.equal(checkDesktopFile("platform/legacySpeechPreferences.ts", 'window.localStorage.getItem("legacy");').length, 0);
  assert.ok(checkDesktopFile("platform/legacySpeechPreferences.ts", 'window.localStorage.setItem("legacy", "new");').length);
});
test("native code cannot depend on UI policy, bypass event typing or hide commands", () => {
  assert.ok(checkNativeFile("src-tauri/src/studio.rs", 'include_str!("../../apps/desktop/src/policy.json")').length);
  assert.ok(checkNativeFile("src-tauri/src/chat.rs", 'app.emit("event", json!({}));').length);
  assert.ok(checkNativeFile("src-tauri/src/chat.rs", '#[tauri::command] fn hidden() {}').length);
  assert.equal(checkNativeFile("src-tauri/src/ipc_events.rs", "app.emit(E::NAME, payload)").length, 0);
});
