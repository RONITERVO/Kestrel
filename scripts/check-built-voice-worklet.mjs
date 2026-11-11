import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

// Development URLs and mocked AudioContexts cannot catch Vite inlining a small worklet.
// Check the actual production artifacts that Tauri packages under its same-origin CSP.
const output = new URL("../apps/desktop/dist/", import.meta.url);
const assets = new URL("assets/", output);
const worklets = readdirSync(assets).filter((name) => /^conversationCapture\.worklet-[\w-]+\.js$/.test(name));
assert.equal(worklets.length, 1, "Package exactly one microphone worklet as a local JS file, never an inline data URL.");
const worklet = worklets[0];
assert.match(readFileSync(new URL(worklet, assets), "utf8"), /registerProcessor\(["']kestrel-conversation-capture["']/);

const html = readFileSync(new URL("index.html", output), "utf8");
const entry = html.match(/<script\b[^>]*\bsrc="(\/assets\/[\w.-]+\.js)"/);
assert.ok(entry, "The packaged application must have a local script entry.");
const entrySource = readFileSync(new URL(entry[1].slice(1), output), "utf8");
assert.ok(entrySource.includes(`/assets/${worklet}`), "The microphone must reference the emitted worklet file.");
console.log("Packaged microphone worklet is a referenced same-origin JavaScript asset.");
