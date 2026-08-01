# Kestrel Local Research

Kestrel is a Windows-first, fully offline research workspace for a local Bonsai model and a local Kiwix Wikipedia archive. It turns a question into a source-traceable research edition, reads comfortably inside the native app, and remains available as plain JSON and self-contained HTML years later.

This branch focuses on one complete experience: **Bonsai 27B + English Wikipedia research**. It intentionally uses a two-tool model harness instead of a general agent framework.

## What works

- Native Tauri application with a responsive React research reader.
- Automatic health detection for the installed services:
  - `Ternary-Bonsai-27B-Q2_0` through the OpenAI-compatible server at `127.0.0.1:8080`;
  - English Wikipedia January 2024 MAXI archive through Kiwix at `127.0.0.1:8085`.
- One-click local service preparation using the existing scripts under `D:\LocalAI`.
- A model-specific native OpenAI tool loop with only:
  - `search_archive` — searches both Kiwix and the Kestrel research catalog;
  - `read_source` — opens an exact Wikipedia result or prior Kestrel report.
- Focused and thorough safe modes, plus an opt-in Solo expedition profile that coordinates many archive lanes inside one shared model context.
- Advanced values for context, answer allowance, lane count, results per lane, source target, tool turns, reasoning budget, and source size are persisted without Kestrel-imposed upper caps.
- A System view restores one-click navigation to live GPU VRAM, loaded-model footprint, runtime context/output/KV state, research tuning, and the existing Bonsai control center.
- Visible six-stage progress, live activity messages, elapsed time, and safe cancellation.
- Source validation: a citation is accepted only when its source was actually opened.
- Safe normalization of small-model Kiwix book-ID copy errors without permitting non-loopback URLs or path traversal.
- Existing-research discovery and immutable edition lineage. Related work is opened and improved; prior editions are never overwritten.
- A durable research library that scales to thousands of reports with SQLite FTS5 while remaining tool-agnostic through JSONL and per-report files.
- Polished in-app reading plus a self-contained, print-friendly HTML page for every edition.
- A compact structured-output retry for the Bonsai-specific case where reasoning leaves an otherwise valid JSON response incomplete.

## Exact local setup targeted first

The tested installation is:

```text
Model API:   http://127.0.0.1:8080/v1
Model:       D:\LocalAI\Bonsai27B\models\Ternary-Bonsai-27B-Q2_0.gguf
Runtime:     D:\LocalAI\Bonsai27B\runtime\llama-server.exe
Kiwix:       D:\LocalAI\OfflineWikipedia\tools\kiwix-tools-3.8.1\kiwix-serve.exe
Archive:     D:\OfflineInternet\wikipedia_en_all_maxi_2024-01.zim
Snapshot:    2024-01-12
Articles:    6,863,660
```

Kestrel reads those assets in place. It does not copy the 102.3 GiB ZIM or model weights.

## High-capacity solo research

The tested server uses one GPU slot at 98,304 context tokens, a 32,768 maximum response allowance, Q4 key/value cache, and roughly 9.7 GiB of measured model/runtime VRAM on a 12 GiB RTX 5070. Starting several model agents would duplicate KV state, queue behind the same slot, and risk exhausting VRAM.

Solo expedition uses the available capacity differently:

1. Bonsai plans complementary research lanes in one model context.
2. Kestrel searches those Kiwix lanes concurrently; this is fast CPU/I/O work and does not create model contexts.
3. Compact candidate references become shared memory. They are not evidence and cannot be cited yet.
4. The same Bonsai context chooses and opens sources, with unreadable archive results treated as recoverable misses.
5. Publication requires the configured number of successfully opened Wikipedia sources. Native code still validates every citation.
6. A larger expedition schema permits substantially deeper reports, and a Bonsai-specific adapter safely normalizes known alternate field names without trusting paths or citations.

Advanced mode is opt-in. Its initial Bonsai-specific values are 98,304 context, 32,768 maximum output, six lanes, six candidates per lane, twelve inspected sources, twenty-four tool turns, 4,096 reasoning tokens, and 20,000 characters per opened source section. Values are deliberately not capped by Kestrel.

> Warning: invalid or oversized values can stop startup or exhaust VRAM. The model runtime and hardware still impose their own limits.

Saving the research profile does not restart the model. **Apply & restart model** writes the selected context/output values to the chosen Bonsai installation, preserves the previous settings as `settings.json.kestrel-backup`, and explicitly restarts its local server. Standard modes continue using their tested internal budgets even when advanced mode is disabled.

## Research storage

The default library is the unsynced local home folder:

```text
C:\Users\<you>\Kestrel Research\
├── README.txt
├── catalog.sqlite3        # fast FTS5 discovery
├── catalog.jsonl          # rebuildable model/tool index
└── reports\YYYY\MM\<title>--<id>\
    ├── index.html         # standalone polished page
    ├── report.json        # full structured report
    ├── sources.json       # inspected evidence ledger
    └── provenance.json    # query, model, snapshot, edition, parent
```

The home folder is used deliberately. On machines where Documents is redirected into OneDrive, this avoids silently placing private offline research in a cloud-synced directory.

SQLite is an acceleration layer, not a lock-in layer. The JSONL catalog and report bundles remain readable by local models and ordinary tools if the database is unavailable. A report ID and its files never change after publication.

## Offline boundary

- The native backend accepts only fixed loopback model and Kiwix endpoints.
- Kiwix is started with `--blockexternal` by the installed script.
- The Tauri WebView content-security policy permits no external network connection.
- No remote provider, browser-search tool, MCP server, analytics SDK, CDN, or web font is configured.
- Wikipedia's January 2024 cutoff is displayed in the app and stored with every source.

Wikipedia is a tertiary starting point. The report format distinguishes the inspected evidence ledger and open questions; it does not describe Wikipedia as a primary source or imply knowledge after the archive date.

## Run from source

Requirements: Windows 10/11, Node.js 20.19+ or 22.12+, Rust stable with the MSVC target, WebView2, and the local services above.

```powershell
npm install
npm run tauri dev
```

If Bonsai or Kiwix is stopped, choose **Prepare services** in Kestrel. The app starts only the existing local server scripts; it does not start the larger Open Computer VM.

## Validation

Fast checks:

```powershell
npm run check
npm test
npm run build
cargo test --manifest-path src-tauri\Cargo.toml
```

Real archive acceptance (Kiwix must be running):

```powershell
cargo test --manifest-path src-tauri\Cargo.toml live_archive_search_and_read -- --ignored
```

Real end-to-end acceptance (Bonsai and Kiwix must be running):

```powershell
cargo test --manifest-path src-tauri\Cargo.toml live_bonsai_research_creates_a_complete_offline_bundle -- --ignored --nocapture
```

Real high-capacity Solo expedition acceptance (several minutes on the tested RTX 5070):

```powershell
cargo test --manifest-path src-tauri\Cargo.toml live_solo_expedition_uses_shared_lanes_and_high_output_budget -- --ignored --nocapture
```

The end-to-end test seeds an earlier report and requires the result to become edition 2, link the prior report, include prior research in its evidence ledger, inspect at least two current local Wikipedia articles, pass citation validation, enter the FTS catalog, and produce standalone HTML. See [RESEARCH_ACCEPTANCE.md](RESEARCH_ACCEPTANCE.md).

## Known boundaries and next improvements

- The initial adapter intentionally targets the exact English January 2024 Kiwix book and Bonsai 27B endpoint above. A settings UI for other archives/models is a later adapter, not a hidden heuristic.
- Kestrel does not update or download Wikipedia. Archive acquisition remains an explicit user operation.
- Wikipedia alone cannot provide primary-source research. A later offline corpus adapter should preserve the same `search_archive` / `read_source` contract while adding local papers, books, or user documents.
- A future maintenance command should rebuild `catalog.sqlite3` entirely from `catalog.jsonl` and report folders; the current data is already sufficient for that rebuild.
- The next visual improvement should add collection/tag views once real library usage shows which organization patterns are useful, rather than imposing a premature taxonomy.

## License

MIT. Bonsai, `llama.cpp`, Kiwix, Wikipedia content, and their bundled components retain their own licenses and provenance.
