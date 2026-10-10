# Where frost stands, and what it would take to be alone

A competitive assessment from a design session (October 2026), kept live on
purpose: the **Scoreboard** at the bottom is meant to be edited as the work
lands, so the distance between the claim and the reality never goes stale.
The finding it builds on — the name-keyed asset table as the seam between
`include_bytes!` shipping and watched-file hot reload — is recorded in
`ARCHITECTURE.md`, Direction.

The claim under test is one sentence:

> Ship 100 % embedded single-binary, develop with the same names against
> watched loose files, iterate on data + media + shaders with zero import
> step and total state preservation, and keep whole-program LTO with the
> game inlined into it — because the engine is a static library, not a
> runtime.

*(Sources: knowledge of the landscape, not a fresh sweep — the session's
web search was down. Re-verify Bevy's plugin status and Bitcraft's health
before quoting this in public.)*

*Where this document says **where** frost stands, `MANIFESTO.md` says
**why** — the amendments this comparison is read through. A scoreboard
rung with no article behind it is drift; an article with no rung is
marketing; both files exist to keep the other honest.*

## The axes

Nobody competes on both ends of the rope:

- **hot** — asset reload, and honestly-labelled code reload, with the
  running app's state intact;
- **static** — compiled like an application, shipped as one embedded
  binary, whole-program optimization with the game.

## The hot-code crowd — they paid in static compilation

| engine | how it gets hot | what the ticket cost |
|--------|-----------------|----------------------|
| Bitcraft (Zig) | bespoke JIT + IR, live code editing, genuinely magical | the JIT *is* the product: no static compilation, no LTO, its own runtime — it bought reload by selling the exact thing frost is |
| Flexio (C) | custom JIT, hot reload as identity | same trade, same price |
| Godot / Unity / Defold / LÖVE | code-as-data: GDScript/C#/Lua run on an interpreter or VM, so reload is free | the code runs on someone's interpreter forever; Unity's fast path (IL2CPP) ships exactly where its reload dies |

None of them ship as one binary. They ship an executable plus a
PCK/bundle directory — the `include_bytes!` single-file story is strictly
beyond them, not equal to them.

## Bevy — "to some extent" is exactly right

The closest cousin, and the most instructive, because it tried the same
trick being contemplated here.

- **Assets:** first-class. `AssetServer` watches files and reloads live,
  no import ceremony. On this axis Bevy genuinely competes — and today
  beats frost, which has the idea, not the code.
- **Code:** the `bevy_ecs/dylib` dynamic-plugins feature — the game
  compiled as a CDylib, swapped at runtime, the host holding the `World`
  across an ABI seam. Bevy's own book introduces it roughly as *this
  hurts; use it to iterate, never ship it*. That is the ABI-tax argument
  confirmed by the engine that grew the scar.
- **Library claim:** marketed library-first, but the mass is real — ECS
  scheduling, reflection, type registration, a long dependency tree and
  long compiles. A library claim about the loop; the rest is still an
  engine wearing a library trench coat.

## The library-first siblings — right philosophy, smaller reach

Penguin (oxyquad), Macroquad, Fyrox lean library-first. Penguin's
data-driven approach rhymes with the data tier hardest: an app that is a
pure function over data hot-reloads trivially, because everything
interesting already *is* a file. None built the release/dev resolver
duality, and none couple reload to a compile-time-embedded shipping model.

## The verdict, and its two shadows

Combination-wise nobody holds both ends: Godot can't ship embedded,
Bitcraft can't be static, Bevy embeds but its dylib dance is
self-documented pain, Unity's reload dies where its performance begins.

Two honest caveats keep the claim honest:

1. **The ladder only wins if the data tier is real.** Hot reload without
   gameplay values living in `.ron` is just live sprites. The data tier is
   the crown jewel and also the cheapest rung.
2. **The single binary has a shadow side nobody markets: update cost.**
   One tweaked tileset rebuilds the whole shipped binary. For games
   (what `immortal` already chose) that is fine; for patch-heavy
   distribution, loose assets win — and because the resolver keeps both
   worlds behind one name, frost can offer *both*, per release, which no
   engine on this list can say in one sentence.

## The truth inversion (why the editor question has no easy answer)

Godot's scene tree *is* the application — truth lives in `*.tscn` files
the editor mutates and the game merely renders. frost inverts it: truth is
a running program, the scene tree is a frame artifact, and no tool can
extract an app's state — it can only be told. The resulting contract
(recorded in full in `ARCHITECTURE.md`, Direction): **documents** are the
app's truth while sleeping (saves, `.map.ron`, gameplay `.ron` — which is
why state lives in RON on purpose), **snapshots** are its truth while
running (the app announces itself and publishes a hand-written RON Val
tree, in its own words, zero cost where absent), tools write documents
and *never* memories, and the very watcher that powers hot reload is the
bus — tool→app for edits, app→tool for state. Godot's bidirectional
scene-editing is the coupling this refuses.

## Scoreboard

Status of each rung, updated as work lands. `[ ]` = claim, `[x]` = fact.

- [x] **Asset names** — `TilesetRef { name, rows, cols }` landed;
      `one_file_cut_two_ways_stays_two_tilesets` pins the dress (the
      path-only identity it replaced dressed both cuts from the first
      same-path slot — the bug the test's slot order keeps as a trap).
      Checked 2026-10-10.
- [x] **Map file** — `.map.ron` codec landed: sparse cells (empties are
      never written), tileset identity stamped per layer, round-trip and
      refusal tests in `map.rs`; `Save map` / `Open map` born in
      `actions.rs` as the first verbs on the seam. Checked 2026-10-10.
- [x] **The table** — `frost::Assets`: an app's `include_bytes!` entries
      as one `frost::Table`, loose directories shadowing it by name;
      immortal serves 44 sprites, sounds, and fonts through the one
      resolver — and its release binary boots from `/tmp` with no
      `assets/` anywhere. Refined on the way: no feature flag — the
      flag would only fork what disk-over-table already unites.
      Checked 2026-10-10.
- [ ] **Data tier** — a gameplay `.ron` re-parsed on change; the running
      app rebuilds config from the new root. *The rung that wins
      against Godot.*
- [ ] **Media tier** — textures/audio/fonts: mtime → decode → re-upload →
      rebind by name, mid-run, state intact.
- [ ] **Shaders** — `.wgsl` watched, pipeline rebuilt at runtime
      (`create_render_pipeline` is already a runtime call; naga is
      already a dependency). The rung where frost embarrasses the big
      engines.
- [ ] **The guard** — `frost::map::tileset_names` + the app-side test
      *the world ships every tileset it names*: a forgotten
      `include_bytes!` dies at `cargo test`.
- [ ] **The telling** — live announce + `state_of_affairs` snapshot in
      the house RON dialect, transition-driven (value-key dirty check,
      frame-end flush, `rev`-counted, structure only — an autosave with
      manners); a tool attaches to a running app and reads its being,
      tools writing documents only.
- [ ] **A demo to prove it** — `immortal` (or a small new example) runs
      from loose names in debug, ships as one embedded binary, and
      survives a live tileset edit mid-run.

**Code hot reload is deliberately off the scoreboard.** The dylib pattern,
if it ever comes, is a documented app pattern, never a frost API —
Bevy's book chapter stands as the reason, recorded above.
