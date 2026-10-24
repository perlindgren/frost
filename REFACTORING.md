# Refactoring review

Overnight review of the larger files, with the four safe splits
executed as branches off `main` and **all four since merged** (each
branch: pure moves, no behavior change, gate-green — `cargo fmt`,
`cargo clippy --all-targets -D warnings`, 218 lib + 44 sprite_util +
101 immortal tests, wasm32 check, doc-link warnings at the `main`
baseline of two). The `refactor/*` branches are kept for review and can
be deleted with `git branch -d refactor/*` at any time.

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
- **`tools/sprite_util/main.rs`** (the former `examples/sprite_util.rs`,
  3789 then; its own workspace package now, the file since grown) — a
  narrated single-file tool on purpose (129-line module doc, 24% comments,
  tests via `super::*`).
  One cheap win if it ever grows again: lift the 266-line paint tail of
  `process()` (the checkerboard/strip/markers block) into a private
  `fn paint`; read-mostly, zero test churn. Everything else should stay
  one file.
- **`diagnostics.rs` HUD core** — flags, lines, placement, and the
  one-step `process` are one product; the charts were its only real
  second responsibility.

## The one big remaining candidate: `examples/immortal/main.rs` — DONE

Immortal already splits its simulations into sibling modules (`bugs`,
`plant`, `worms`, `fall`, `wasps`); main.rs has since shed its three
biggest remaining systems (branch `refactor/immortal-split`, merged):

1. **`tools.rs`** (~1,076 lines): the `Tool` enum, per-tool transforms
   (can, spray, spade, tweezers), the bay/slot/held-cell geometry, swap
   arcs, particle consts, and the eleven driving `Demo` methods. In a
   binary crate a module may extend the root's types, so the methods
   moved as an `impl Demo` block with no visibility churn beyond
   `pub(crate)`.
2. **`fruit.rs`** (~658): `Pick`/`Fly`/`Landing`, the carry/flight/
   fall/planting methods, tomato pick geometry.
3. **`scene.rs`** (~313): main's literal scene tree as
   `build_scene(&Assets)`, with the `CHILD_*` index contract next to
   the tree.

main.rs: 5151 → 3160. Left in place deliberately: the `process()` call
sequence (the documented frame-invariant contract), `handle_input`
(input orchestration), the save/capture/restore glue (touches every
field), and the HUD/overlay (its `restart` reaches everywhere). The
plants-and-water driver (~300, thin over `plant.rs`) remains the only
runner-up.

## The antifrost binary split — DONE (branch `antifrost_refactor`)

The crate's domain half (`game`, `level`, `ledger`, `settings`,
`route`, the solver) was already exemplary; the whole presentation
half sat in one 8,168-line `main.rs`. Six pure-move branches off
`main`, landed as one branch, same rules as the immortal split —
`use crate::*` modules, `pub(crate)` at the seams, no behavior change
anywhere. Gate-green end to end: `cargo fmt`, workspace clippy at zero
warnings (the pre-commit hook ran it per commit), the same 21 lib +
61 bin tests before and after, `cargo check --target
wasm32-unknown-unknown -p antifrost`, `--features diag`, and doc-link
warnings at the `main` baseline of one (`ambient_floor`, pre-existing).

| move | effect |
|---|---|
| `shadow.rs` (427) — the dig watch, `OccBox`, `seed_cover`/`merge_boxes`, `dig_tests`, `merge_tests` | the cover math is pure cell functions with its proofs beside it |
| `cli.rs` (145) — `Cli`, `parse_on_off`, `init_logging`, `now_ms`, `report_gap`, `cli_tests`; `main()` stays at the front door | the binary's plumbing reads as plumbing |
| `lessons.rs` (635) — `Lesson`, every authored script array, the tour state machine (`warmed`/`cold_tour`/`dig_tour`/`oopsy_visit`/`script`/`rail_opens`) and the rail's marks | the hill's teaching is content, not renderer |
| `window.rs` (438) — `fit_16x9`, `TierAsk`, `apply_window_policy`, `draw_letterbox`, and the ~200-line compositor watch lifted out of `process()` into `watch_window` | the frame loop reads as an orchestration again |
| `menu.rs` (2,809) — `Page`/`Escape`/`Action`/`Row`/`Slider`, the menu geometry, the rows a page presents, its input and painting, and the 1,428-line `menu_tests` module beside what it tests | the biggest single win, and the tests finally live where the code does |
| `world.rs` (1,237) — `Hop`/`Zzz`/`Splash`/`Glide`/`Leaf`/`Sky`, the weather, breath, zs, shadows, ant painting, the verdict splash | the living hill, one file |

`main.rs`: 8,168 → 2,586. Left in place deliberately: the asset
constants (one reading list for the hill's look), the `Demo` struct,
`load`, camera, gameplay input, `refresh`, the replay walker, the HUD,
and `process()`'s frame contract. The ~65-field `Demo` is now the one
remaining candidate — field clusters (`MenuNav`, `WindowWatch`,
`Weather`) could group — but that is not a pure move and waits for its
own branch.
