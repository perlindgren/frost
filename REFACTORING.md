# Refactoring review

Overnight review of the larger files, with the four safe splits already
executed as branches off `main` (each branch: pure moves, no behavior
change, gate-green — `cargo fmt`, `cargo clippy --all-targets -D
warnings`, 218 lib + 44 sprite_util + 101 immortal tests, wasm32 check,
doc-link warnings at the `main` baseline of two). Merge any, all, or
none; rebase is trivial since the branches touch disjoint files.

## Branches

| branch | move | effect |
|---|---|---|
| `refactor/canvas-module` | `Canvas` + the scene-expansion machinery (`draw_node`, `expand_text_list`, repeat offsets, cone/feather math) + their 24 tests, out of `lib.rs` → `src/canvas.rs` | `lib.rs` 2944 → 678 (docs, module wiring, `Context`, `Config`, `run`); the crate root becomes the front door again |
| `refactor/backend-windowing` | `create_window`, `center_on_screen`, `present_mode_for`, `fit_into_monitor`, `fit_bytes`, `content_fit` + their tests → `src/backend/windowing.rs` | `app.rs` 2221 → 1971; the pure window geometry is one readable file with its tests beside it |
| `refactor/diagnostics-charts` | the chart data model (`ChartSlot`, `Sample`, `chart_points`, `trim_samples`, `slice_max`, `series_*`) + their 10 tests → `src/diagnostics/charts.rs` | `diagnostics.rs` 1755 → 1463; the HUD keeps one responsibility; `src/diagnostics.rs` + `src/diagnostics/charts.rs` is the same submodule pattern as `backend/` |
| `refactor/ui-tree-tests` | the 554-line test module of `ui/tree.rs` → `src/ui/tree/tests.rs` | widget 1503 → 951; the widget code itself untouched |

Each branch kept visibility crate-internal: a few moved helpers became
`pub(crate)`/`pub(super)` where the module boundary required it — no
public API change anywhere.

## Deliberately not split

- **`src/objects.rs` (1258)** — one cohesive vocabulary module (color,
  transform, shapes, nodes, scene), ~45% doc comments, two tests,
  re-exported wholesale. Splitting adds directories, not clarity.
- **`ui/tree.rs` widget code** — the tree's claim/tolerance state
  machine and its painter share documented geometry semantics; the
  file's only structural problem was its test block, now moved.
- **`ui/mod.rs` (1110)** — `Ui` core + six small widgets on shared
  plumbing. Optional tidy: its test module tests three files (core,
  `panel.rs`, `table.rs`); the table/panel tests could move beside what
  they test. Cosmetic, low value.
- **`examples/sprite_util.rs` (3789)** — a narrated single-file tool on
  purpose (129-line module doc, 24% comments, tests via `super::*`).
  One cheap win if it ever grows again: lift the 266-line paint tail of
  `process()` (the checkerboard/strip/markers block) into a private
  `fn paint`; read-mostly, zero test churn. Everything else should stay
  one file.
- **`diagnostics.rs` HUD core** — flags, lines, placement, and the
  one-step `process` are one product; the charts were its only real
  second responsibility.

## The one big remaining candidate: `examples/immortal/main.rs` (5151)

Immortal already splits its simulations into sibling modules (`bugs`,
`plant`, `worms`, `fall`, `vipers`); `main.rs` still holds two systems
in the old pattern:

1. **`tools.rs`** (~1,100–1,450 lines with tests, biggest win): the
   `Tool` enum, per-tool transforms (can, spray, spade, tweezers), the
   bay/slot/held-cell geometry, swap arcs, and the tool half of
   `handle_input`. Mostly stateless consts + free functions — the
   `basket.rs`/`zorder.rs` pattern — so it can move in stages; the
   22 `Demo` fields behind it would fold into a `Tools` struct with
   `step(dt)`/`layout(node)` per the sim-pool convention (like
   `bugs.rs`), leaving the particle taps (`water.particles`,
   `spray.particles`) and sound calls in `main`.
2. **`fruit.rs`** (~570): pick/carry/drop/planting flight and the fall
   drivers; its one coupling (`Landing::Slot` writes `plants[i]`)
   becomes a returned event exactly like `bugs::StepEvents`.
3. **`scene.rs`** (~300, near-zero risk): `main()`'s literal scene-tree
   block into `build_scene(&Assets)`, keeping the `CHILD_*` index
   contract next to the tree.

Left undone overnight: this is the user's active demo, the `Demo`
field-grouping is design taste rather than a mechanical move, and
`process()`'s call order is a documented frame-invariant contract best
kept visible in `main` — all reasons to decide it with fresh eyes
rather than at 3 a.m. The branches above need no such judgement: they
are pure relocations, reviewable as single moves.
