# Visual lyric rendering contract

Kestrel carries two offline, code-owned visual lyric themes over one durable lyric/playback
surface. The visual layer may react to the preserved master and current word, but it cannot alter
audio, timing truth, or the immutable take.

## Reactivity contract

`MusicLyricReactivity.ts` is the only analyser sampler. It reads one producer-owned 1024-point Web
Audio analyser and publishes a bounded frame containing sub-bass, bass, low-mid, mid, presence,
air, RMS loudness, positive spectral flux, transient envelope, spectral centroid, 48 smoothed
display bands, waveform, song progress, and lyric geometry. Both renderers and the lyric CSS use
that same frame. Do not add a renderer-specific analyser or a second media source node.

The buckets approximate musical frequency regions as fractions of Nyquist instead of fixed bin
numbers. This keeps their meaning stable when analyser resolution changes. Attack and release are
smoothed separately so kicks remain immediate while scenery settles rather than flickers.

## Living sketchbook parity and extensions

The original Visual Music Lyrics scene included an audio-shaped scribble sun, watercolor clouds,
parallax terrain, waveform ocean ripples, broken reflection, rain, splashes, birds, fish, lyric-aware
horizon placement, translated-text collisions, handwriting, erasure, and reflected translation.
Kestrel keeps those behaviors and adds:

- song-progress travel for the sun;
- two independently scrolling terrain depths;
- separate musical-band ownership for sun, terrain, clouds, water, wind, and weather;
- deterministic event generation and strict caps of 420 drops, 96 splashes, five birds, and one
  fish instead of frame-rate-dependent unbounded random arrays;
- a single measured lyric-layout contract rather than global document queries; and
- reduced-motion behavior for lyric transitions.

## Signal bloom extensions

Signal bloom is Kestrel-original. Its nocturnal grid, spectral ribbons, travelling bloom, waveform,
constellation, and progress journey now also bind to the current word. Positive spectral flux and
bass onset emit bounded shockwave and spark pools; spectral centroid influences their hue; presence
controls glow and ribbons; air controls constellation detail. The pools are allocated once (72
sparks and six pulses) and reused.

## Typography and interaction

Timed words remain real buttons with exact seek positions. Each button has a stable ghost layer and
a clipped ink layer, revealing the word continuously over its timestamp rather than changing color
only after the boundary. Untimed primary and translated lines receive deterministic cue-relative
word staging. A completed cue gets a 420 ms visual erasure grace period without changing the strict
timing lookup used elsewhere. Theme motion never removes the button or shrinks its hit target.

When changing this system, preserve keyboard names, word seeking, translated-text readability,
reduced-motion behavior, fixed collection limits, one analyser, and idle animation when no analyser
is available. Test both themes with silent, bass-impact, vocal-presence, and high-air frames and at
the desktop and 900 px breakpoints.
