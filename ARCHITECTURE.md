# Kestrel Research Architecture

## Product invariants

1. Research works with the public network unavailable.
2. The WebView never talks directly to model or archive services.
3. Native HTTP targets are fixed loopback addresses.
4. Only sources actually opened by the harness can be cited.
5. A published edition is immutable; expansion creates a linked child edition.
6. HTML/JSON remain useful without Kestrel or SQLite.
7. The UI names the model and archive cutoff and never calls a report final or best possible.

## Smallest useful process boundary

```text
Tauri WebView
  │ typed IPC + progress events
  ▼
Rust research controller
  ├── two-tool Bonsai harness
  ├── citation/evidence validator
  ├── edition matcher
  ├── SQLite FTS5 catalog
  └── JSON + self-contained HTML publisher
        │
        ├── authenticated/local OpenAI API → 127.0.0.1:8080
        └── Kiwix search/article HTML      → 127.0.0.1:8085
```

There is no general shell tool, browser tool, MCP client, subprocess agent, remote provider, or second model in the research loop.

## Bonsai-specific harness

The installed 27B generation supports native OpenAI-style `tool_calls` when `llama-server` runs with `--jinja`. Kestrel therefore sends standard tool schemas and round-trips tool results without prompt-format hacks.

The stable tool prefix contains two functions:

| Tool | Purpose |
| --- | --- |
| `search_archive(query, limit)` | Returns compact matches from prior Kestrel research and local Wikipedia. |
| `read_source(source_ref, section, max_chars)` | Opens one exact result, records evidence, and returns a bounded excerpt. |

Focused mode requires at least two distinct Wikipedia articles; thorough mode requires four. Tool loops are bounded to 9 and 14 turns respectively. Parallel tool calls are disabled for predictable single-slot inference.

Final synthesis uses a strict JSON schema. Field lengths are bounded for readable pages. If the first structured response is incomplete, Kestrel performs one compact retry with thinking disabled for that retry only. The normal research/tool calls retain a reasoning budget.

## Evidence and citations

An evidence ID is allocated only by `read_source`. Search results are not evidence. On publication every citation array is intersected with the evidence ledger; unknown IDs are removed, duplicates collapse, and an uncited content block receives a valid inspected-source fallback.

Kiwix references must resolve to `http://127.0.0.1:8085/content/...`. The archive book segment is canonicalized to the configured book because small local models sometimes copy one digit incorrectly. Scheme, host, port, path traversal, query strings, fragments, backslashes, control characters, and excessive length remain rejected.

## Existing-research decision

FTS5 produces candidate reports. A deterministic keyword-overlap floor prevents unrelated parent links. The best qualifying report becomes `parentId`; the new edition number is parent + 1. Its answer is added to the evidence ledger and the model receives all close catalog matches. The final schema requires a concrete `improvement` and open questions.

This is deliberately hybrid: deterministic code owns identity and lineage, while the model decides how the evidence should improve the explanation.

## Durable publication

Publication writes to a new ID-based folder and then inserts catalog metadata. Each report contains:

- `report.json`: full UI document;
- `sources.json`: opened evidence only;
- `provenance.json`: model, archive, query, parent, edition, improvement, offline flag;
- `index.html`: escaped, self-contained page with embedded report metadata and print CSS.

`catalog.sqlite3` uses WAL mode and FTS5. `catalog.jsonl` is regenerated in updated order after each publication. The file formats are intentionally simple so a future local LLM can find and open any edition without Kestrel-specific APIs.

## Progress and cancellation

The backend emits six named stages: prepare, library, search, read, synthesize, publish. Each event includes a human-readable detail and elapsed seconds. The client keeps elapsed time moving between model responses. Cancellation uses a token checked before each tool/model step and races every long model request; already-published reports are never partially overwritten.
