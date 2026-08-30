import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const committedRoot = join(repositoryRoot, "packages", "generated-bindings", "src");
const check = process.argv.includes("--check");
const outputRoot = mkdtempSync(join(tmpdir(), "kestrel-bindings-"));

function listFiles(root, cursor = root) {
  return readdirSync(cursor, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(cursor, entry.name);
      return entry.isDirectory() ? listFiles(root, path) : [relative(root, path).replaceAll("\\", "/")];
    })
    .sort();
}

function writeIndex(root) {
  const exports = readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".ts") && entry.name !== "index.ts")
    .map((entry) => basename(entry.name, ".ts"))
    .sort()
    .map((name) => `export type { ${name} } from "./${name}.js";`);
  writeFileSync(
    join(root, "index.ts"),
    `// Generated from Rust by scripts/generate-bindings.mjs. Do not edit.\n${exports.join("\n")}\n`,
  );
}

function normalizeGenerated(root) {
  for (const path of listFiles(root).filter((name) => name.endsWith(".ts"))) {
    const absolute = join(root, path);
    const normalized = readFileSync(absolute, "utf8")
      .replace(/[ \t]+$/gm, "")
      .replace(/\s+$/, "\n");
    writeFileSync(absolute, normalized);
  }
}

try {
  execFileSync("cargo", ["test", "-p", "kestrel-app-core"], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      TS_RS_EXPORT_DIR: outputRoot,
      TS_RS_IMPORT_EXTENSION: "js",
      TS_RS_LARGE_INT: "number",
    },
    stdio: "inherit",
  });
  normalizeGenerated(outputRoot);
  writeIndex(outputRoot);

  if (check) {
    const expected = listFiles(outputRoot);
    const actual = listFiles(committedRoot);
    const names = [...new Set([...expected, ...actual])].sort();
    const changed = names.filter((name) => {
      if (!expected.includes(name) || !actual.includes(name)) return true;
      return readFileSync(join(outputRoot, name), "utf8") !== readFileSync(join(committedRoot, name), "utf8");
    });
    if (changed.length) {
      throw new Error(`Rust/TypeScript bindings are stale:\n${changed.map((name) => `  ${name}`).join("\n")}\nRun npm run bindings:generate.`);
    }
  } else {
    rmSync(committedRoot, { recursive: true, force: true });
    mkdirSync(committedRoot, { recursive: true });
    cpSync(outputRoot, committedRoot, { recursive: true });
  }
} finally {
  rmSync(outputRoot, { recursive: true, force: true });
}
