# The Kestrel book

Kestrel is one bound sketchbook on a desk. Each workspace is a chapter behind a labelled index
tab. Pencil lines and watercolor washes come from Ink Battle's spatial sketchbook, so Kestrel and
the game share one visual language.

## Why

The previous interface had three problems. Each tab had its own theme, dark or light, and the
stylesheets held about 1,700 hard-coded colors. Text was often 7–9 px. Page scrolling hid options,
so a screenshot could not show what a page offered.

## Rules

1. **One palette.** Colors live only in `apps/desktop/src/app/book/tokens.css`.
   `npm run ui:check` rejects raw colors in any other stylesheet or inline style. Each color has
   one meaning in every chapter:
   - `act`: the next action or a selection.
   - `ok`: ready or done.
   - `wait`: working or needs attention.
   - `stop`: an error or a destructive action.
   - `note`: information.

   A chapter's ribbon hue marks where you are. It never shows state.
2. **Four typefaces.**
   - Caveat: titles.
   - Patrick Hand: labels and controls.
   - Georgia: long reading text.
   - Cascadia Mono or Consolas: paths and data.

   The smallest text size is 12 px. Both handwriting fonts are bundled, so nothing loads from
   the network.
3. **Nothing hides below the fold.** Pages do not scroll. Long content flows onto turnable pages
   (`shared/book/FlowPages`), and lists show whole items a page at a time
   (`shared/book/PagedList`). Every page shows "page n of m". Pages break the way a word processor
   keeps paragraphs: a paragraph, list item, table or code block that fits on a page moves to the
   next page whole, headings stay with the text after them, and a block taller than a page splits
   with at least three lines on each side. Every page keeps one free line at its foot; when a reply
   or paragraph carries on, that line says "continues". The model's text is never rewritten to
   make it paginate. Only four things may scroll inside themselves:
   - free-text editors;
   - time axes, such as timelines and piano rolls;
   - the media bin of the Studio edit room, which is part of the editor's time-axis tools;
   - live logs that follow their newest line.
4. **A screenshot explains itself.** The index tabs are always labelled. Running heads name the
   chapter. The banner writes out every status in words. Save and render actions sit in a page
   footer so they are never on a hidden page.
5. **Spread grammar.** The left page is for choosing: libraries, contents and settings. The right
   page is for work: the document, conversation or form. Editors that need width (Studio edit,
   Image, Music) span both pages.
6. **Rendered only when something changes.** The WebGL book draws a frame only when something
   changes, so it uses no GPU time while idle and never competes with the local model. three.js
   loads lazily. Reduced motion, jsdom and lost WebGL contexts fall back to the same book drawn
   in CSS.

## How it works

- **DOM pages.** `app/book/BookShell.tsx` owns the DOM plane: pages, tabs and rings. The DOM
  layout decides the geometry, and every control stays a real, focusable element.
- **WebGL body.** `app/book/bookScene.ts` draws the cover, page block, ribbon, corner rings,
  contact shadow and the turning leaf around that plane. Depth-only occluders let the DOM show
  through.
- **Desk pose.** "Lay on desk" tilts the book. `homography.ts` projects the live DOM onto the
  3D page with `matrix3d`, so the pages remain usable at any angle. Drag the desk to orbit, drag
  a ring to turn, and press Esc to return to reading.
- **Chapter turns.** A curling leaf crosses the spine while the outgoing chapter stays on the near
  page until the leaf covers it.

## Mixed reality (future work)

The book already uses physical proportions, drawn carrying rings, labelled targets, no hover-only
controls and no scrolling. Those are the interaction rules Ink Battle uses on Quest. Rendering
Kestrel in a headset is a separate decision. The pages would need to be drawn into textures, and
the headset would need some way to reach Kestrel. Any design for that must keep the offline and
loopback-only rules in `AGENTS.md`.

## Ink Battle as a chapter (future work)

The game can become a turnable chapter as separate work. Ink Battle's Gemma opponent runs in the
browser through LiteRT-LM and WebGPU. Kestrel must not add a second model runtime: a game chapter
would send short, tool-free requests through `RuntimeManager` and its single inference slot.
