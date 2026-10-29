// Thin transport only. Rust owns contracts, access, retries, jobs, and workflow execution.
// node scripts/producer-control.mjs SESSION.json REQUEST.json
// A request filename of '-' reads a single JSON object from stdin.
import { readFileSync } from "node:fs";
import { connect } from "node:net";
import { randomUUID } from "node:crypto";

const [sessionPath, requestPath] = process.argv.slice(2);
if (!sessionPath || !requestPath) {
  console.error("Usage: node scripts/producer-control.mjs SESSION.json REQUEST.json (or - for stdin)");
  process.exit(2);
}
const session = JSON.parse(readFileSync(sessionPath, "utf8").replace(/^\uFEFF/, ""));
const request = JSON.parse(readFileSync(requestPath === "-" ? 0 : requestPath, "utf8").replace(/^\uFEFF/, ""));
if (session.protocolVersion !== 1 || !session.pipe.startsWith("\\\\.\\pipe\\kestrel-producer-")) throw new Error("Unsupported Kestrel producer session.");
request.requestId ??= randomUUID();
request.args ??= {};
const wire = JSON.stringify({ token: session.token, request }) + "\n";
if (Buffer.byteLength(wire) > 8 * 1024 * 1024) throw new Error("Request exceeds 8 MiB.");
const socket = connect(session.pipe);
let output = "";
socket.setEncoding("utf8");
socket.on("connect", () => socket.write(wire));
socket.on("data", (chunk) => {
  output += chunk;
  if (Buffer.byteLength(output) > 8 * 1024 * 1024) socket.destroy(new Error("Response exceeds 8 MiB."));
});
socket.on("end", () => {
  try {
    const response = JSON.parse(output);
    console.log(JSON.stringify(response, null, 2));
    if (!response.ok || response.result?.status === "failed") process.exitCode = 1;
  } catch (error) { console.error(`Incomplete producer response: ${error.message}. Retrieve receipt ${request.requestId} before retrying.`); process.exitCode = 1; }
});
socket.on("error", (error) => { console.error(`${error.message}. Retrieve receipt ${request.requestId} before retrying an uncertain action.`); process.exitCode = 1; });
