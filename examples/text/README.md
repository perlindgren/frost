# Text examples

Four demos of frost's text shapes — `frost::Shape::Text`, created with
`Shape::text` (from a font file on disk) or `Shape::text_bytes` (from font
bytes in memory): TTF glyphs shaped, rasterized into a glyph atlas, and
drawn through the scene tree as sprite quads.

| Example | What it shows | Run with |
|---|---|---|
| `text` | the basic shape: "Sub" set in `assets/fonts/JameGem08_2026-Regular.ttf` at 48 px on top of a swaying circle — the text node is a child of the circle node with the identity transform, so the text rides the circle like any other node | `cargo run --example text` |
| `text_leo` | the same swaying scene with a different font: "immortal tomato 0123456789" in `assets/fonts/Leofont-Regular.ttf` | `cargo run --example text_leo` |
| `sub_zero` | per-letter animation: "SUB0" with the S, U, and B stacked around the center and a six-times-size 0 at the lower left; each letter is its own text node whose `modulate` alpha sweeps 0 → 1 → 0 over one second, 120° out of phase with the others (a raised cosine) | `cargo run --example sub_zero` |
| `text_web` | `text` built for the web: the font cannot be read from a browser, so it is embedded into the binary with `include_bytes!` and created with `Shape::text_bytes`; the full wasm32 + wasm-bindgen build recipe is in its module docs | see the module docs (wasm32) |

The four files sit in this folder rather than at the examples' top level,
so each one is declared explicitly in `Cargo.toml` (cargo only
auto-discovers `examples/*.rs` and `examples/*/main.rs`); the example names
are unchanged. `text_web` targets wasm32-unknown-unknown; when cargo
builds it for a native target instead, its wasm-specific parts are cfg'd
out and it compiles to an empty binary.

## The model

`Shape::text(path, text, size)` reads the font file immediately and
`Shape::text_bytes(font, text, size)` takes font data already in memory
(for example bytes embedded with `include_bytes!` — how text shapes are
created in environments without a file system, such as a web browser).
Both check the bytes up front to be a TrueType/OpenType font, so a missing
file or a non-font fails at creation, not at render time
(`TextError::Io` / `TextError::InvalidFont`). The font bytes live behind
an `Arc`, so every text shape that loaded the same file shares one buffer,
and the render pipeline caches one glyph atlas per (font, size) pair.

A text node is drawn with texture quads, not immediate fills: the text is
shaped into positioned glyphs (swash), every unique glyph is rasterized
once per (font, size) pair and packed into a shared 512-pixel shelf
atlas (a glyph that does not fit is skipped), and each placed glyph
becomes a sprite quad sampling its cell in the atlas — so text inherits
everything the sprite pipeline gives it: modulate, alpha, z-sorting.
Glyph positions are in font pixels at the layout size, y up, relative to
the text's pen origin (the left end of the baseline); at draw time each
glyph's quad is centered on its ink box and the whole text block — its
laid-out width by its ascent plus descent — is centered on the text
node's origin, so a text node with the identity transform at the window
center has its text centered on the screen, as in `text`.

## Patterns the examples share

- **The font path is pinned to the crate root.** The native examples load
  via `CARGO_MANIFEST_DIR` (a compile-time environment variable), so they
  work from anywhere; `text_web` instead embeds the same font's bytes
  into the binary with `include_bytes!` — a path relative to the *file*,
  which is why its `../../assets/...` prefix follows the folder this file
  lives in.
- **Text is a node, so it rides its parent's transform.** In `text`,
  `text_leo`, and `sub_zero` the text nodes are children of a circle
  node, and `process()` nudges the *line* (or the circle) each frame —
  the text moves exactly like any other shape in the tree.
- **Per-letter animation is one node per letter.** `sub_zero` sets each
  character of "SUB0" as a separate text shape in a separate child node,
  so the per-frame `modulate` (white, animated alpha) fades each letter
  independently — 120° phase offsets make the sweep read as three
  channels chasing each other.
