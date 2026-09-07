import ts from "typescript";
import { posix } from "node:path";

const capabilities = new Set(["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "Worker", "SharedWorker", "localStorage", "sessionStorage", "indexedDB", "caches"]);

/** Static guardrails for accidental boundary drift, not a sandbox for hostile source code. */
export function checkDesktopFile(path, source, generatedNames = new Set()) {
  path = path.replaceAll("\\", "/");
  const failures = [];
  const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, path.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const layer = path.split("/")[0];
  const test = /\.(test|spec)\.[cm]?[tj]sx?$/.test(path) || path.startsWith("test/") || path === "test-setup.ts";
  const transport = path === "platform/transport.ts";
  const facade = path === "contracts/index.ts";
  const legacy = path === "platform/legacySpeechPreferences.ts";
  const fail = (message) => failures.push(`${path}: ${message}`);

  function checkImport(value) {
    if (value.includes("@kestrel/generated-bindings") && !facade) fail("Import generated data through contracts/index.");
    if (value.startsWith("@tauri-apps/") && !transport && !test) fail("Only platform/transport may import the native SDK.");
    if (/^(?:node:|fs$|child_process$|electron$|https?$|net$|ffi)/.test(value)) fail("Operating-system and network authority belongs in Rust.");
    if (!value.startsWith(".")) return;
    const target = posix.normalize(posix.join(posix.dirname(path), value));
    const targetLayer = target.split("/")[0];
    if (target.startsWith("../") && !test) fail("UI modules cannot import outside desktop src.");
    if (test) return;
    if (layer === "platform" && !["platform", "contracts"].includes(targetLayer)) fail("Platform cannot depend on application composition or UI features.");
    if (layer === "shared" && !["shared", "contracts"].includes(targetLayer)) fail("Shared presentation cannot depend on features, app, or native transport.");
    if (layer === "features" && !["features", "shared", "platform", "contracts"].includes(targetLayer)) fail("Features cannot depend on app composition or preview fixtures.");
    if (targetLayer === "platform" && layer === "features" && !/^platform\/api(?:\.ts)?$/.test(target)) fail("Features use platform/api, never the transport or migration bridge.");
    if (targetLayer === "preview" && layer !== "preview" && path !== "app/main.tsx") fail("Only the development entry point may load preview code.");
  }

  if (layer === "contracts") {
    for (const statement of tree.statements) {
      if (!ts.isExportDeclaration(statement) || !statement.moduleSpecifier || !ts.isStringLiteral(statement.moduleSpecifier)
        || statement.moduleSpecifier.text !== "@kestrel/generated-bindings") fail("The contract facade contains only re-exports from Rust-generated bindings; no quarantine or local declarations.");
    }
  }

  function visit(node) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) checkImport(node.moduleSpecifier.text);
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      if (node.arguments.length !== 1 || !ts.isStringLiteral(node.arguments[0])) fail("Dynamic imports need a static module path for boundary checking.");
      else checkImport(node.arguments[0].text);
    }
    if (!test) {
      const identifier = ts.isIdentifier(node) ? node.text
        : ts.isElementAccessExpression(node) && ts.isStringLiteral(node.argumentExpression) ? node.argumentExpression.text : null;
      if (capabilities.has(identifier) && !(legacy && identifier === "localStorage")) fail(`${identifier} is outside the UI's authority; add native IPC instead.`);
      if (identifier === "__TAURI_INTERNALS__" && !transport && path !== "app/main.tsx" && path !== "platform/api.ts") fail("Native internals are confined to the platform boundary.");
      if ((ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node)) && generatedNames.has(node.name.text)) fail(`${node.name.text} duplicates a Rust-generated contract. Import it or name this as a distinct view/draft type.`);
      if (legacy && ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
        && ["setItem", "removeItem", "clear"].includes(node.expression.name.text)) fail("Legacy preference migration is read-only.");
    }
    ts.forEachChild(node, visit);
  }
  visit(tree);
  return [...new Set(failures)];
}

export function checkNativeFile(path, source) {
  const failures = [];
  if (/apps[\\/]desktop[\\/]src|packages[\\/]generated-bindings/.test(source)) failures.push(`${path}: Rust cannot consume UI-owned definitions.`);
  if (path !== "src-tauri/src/ipc_events.rs" && (/\bEmitter\b/.test(source) || /\.emit(?:_to|_filter)?\s*\(/.test(source))) failures.push(`${path}: Emit through ipc_events with a Rust event marker.`);
  if (path !== "src-tauri/src/lib.rs" && /#\[tauri::command/.test(source)) failures.push(`${path}: Register commands in lib.rs so signature generation covers them.`);
  return failures;
}
