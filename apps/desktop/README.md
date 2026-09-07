# Working on the Kestrel UI

You can change layouts, styling, navigation, controls, animation, accessibility, and draft editing
without reading or compiling Rust. Start here rather than in `src-tauri`.

From the repository root:

```powershell
npm ci
npm run dev
```

Open the address printed by Vite. The development browser uses sample data, including selectable
movie, image and music projects. Add `?preview=empty` to work on empty libraries. Its unsupported
actions explain that they need the desktop application. It does not start models, write a research
library, or generate media. Fixtures are loaded only by the development entry point, and the
desktop refuses to install a preview adapter.

For a UI change, run:

```powershell
npm run ui:check
npm test -- --run
npm run build
```

These commands use the committed generated bindings and do not need Rust. The full repository
checks additionally compile Rust, verify that generated files are current, and exercise native
behavior. A change to application behavior needs that full verification before merging.

## Where to edit

| Location under `src` | Purpose |
| --- | --- |
| `app` | Navigation, window composition, and global CSS. |
| `features/research` | Report reading and narration controls. |
| `features/workspace` | Chat, Computer Tasks, attachment and transcript views. |
| `features/studio/movie` | Story and scene forms, references, timeline drafts and render views. |
| `features/studio/image` | Composition canvas, project forms and contact sheet. |
| `features/studio/music` | Arrangement, lyrics, MIDI and visualizers. |
| `features/speech` | Microphone interaction, playback, voice forms and device views. |
| `features/control`, `features/setup` | Settings forms, local runtime status and setup views. |
| `shared` | Presentation components and pure view helpers usable across features. |
| `preview` | Development-only fixtures and command handlers for browser work. |
| `platform/api.ts` | The small named API functions that components call. |
| `contracts/index.ts` | Public imports for Rust-generated data. Never define types here. |

Feature components may use other UI features. `shared` cannot depend on features; neither features
nor platform can depend on `app`. Only the platform transport imports the Tauri SDK. The architecture
check explains the offending file and boundary if an import goes in the wrong direction.

## Reading and editing data

Import a shape from the facade, then use it in props or an explicitly unsaved draft:

```tsx
import type { MovieEdit, MovieProject } from "../../../contracts/index";
import { saveMovieEdits } from "../../../platform/api";

// `project` is the last native snapshot. `draft` is what the user is editing.
async function save(project: MovieProject, draft: MovieEdit) {
  const saved = await saveMovieEdits(project.id, draft);
  // Adopt `saved` as the new snapshot; surface a rejected promise to the user.
  return saved;
}
```

Forms may hold incomplete text, selected tabs, drag positions, undo history, pasted proposals, or a
copy of an editable project. That is view state. A save becomes true only when Rust accepts it and
returns a new snapshot. A render becomes complete only when native state says it is complete.
Do not derive completion from a timer, a preview frame, a model's prose, or the last progress step.

`ImageProjectEdit` and `MusicProjectEdit` list their saveable fields separately from the full
project. Generated takes, paths, hashes, receipts, operation status and timestamps are native-owned.
Movie edit and producer requests likewise expose the specific producer action. A whole project
shown in a component is not permission to rewrite its durable fields.

For a new label or interaction, use existing data and API calls. For a new saved setting, command,
event, capability, or generation rule, ask a backend contributor to add the Rust contract first.
They run `npm run bindings:generate`; you then import the new shape normally. Never repair a type
error by editing `packages/generated-bindings`, adding a duplicate interface, or asserting a
different result type on an IPC call.

## Finding an API without knowing Rust

Use editor completion on `platform/api.ts` and on the generated types. The complete native catalog
is in:

- `packages/generated-bindings/src/DesktopCommands.ts`: every registered command, its named
  arguments and successful result. Failures reject the promise with the native error message.
- `packages/generated-bindings/src/DesktopEvents.ts`: every event name and its payload.
- Other generated files: each data shape, including comments from its Rust definition.

The catalog is derived from the actual Tauri function signatures. It is not a second hand-maintained
list. Native emission checks the event registry's payload type; UI subscriptions use that same
generated map. The transport type checks contain examples that must fail compilation, so losing
argument, result or event checking also fails `ui:check`.

## Boundaries that stay in the UI

Browser media APIs, canvas drawing, clipboard interaction, drag calculations, text formatting,
selection, and unsaved undo history belong here. Runtime policy limits and saved speech defaults
are generated values; use those imports for controls instead of inventing another policy table.
Client-side input hints improve editing, while native validation decides what can be saved or run.

See [the full boundary audit and exception register](../../docs/UI_BOUNDARIES.md) for the reason and
native authority behind each of these cases. It also explains opaque third-party receipts and
raw-text editors. None of the exceptions permits UI storage of application records or direct calls
to model servers, ComfyUI, the filesystem, process launchers, or public networks.
