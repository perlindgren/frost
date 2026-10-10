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
- [x] **Data tier** — `frost::Watch` + `Config::from_store`: immortal
      watches `assets/game/gameplay.ron` and rebuilds its config from
      the new root the frame the file changes — proven live: a touch
      at 10:27:35 answered in the log the same second, window up,
      game state intact. Refusals keep the last honest config and
      name themselves. Checked 2026-10-10.
- [x] **Media tier** — landed in immortal's `rebind`, engine additions
      zero (the texture cache had keyed on `(pointer, generation)`
      from the start — new pixels were always a new texture; only
      noticing was missing). Proven live in one second at 08:30:42:
      grass redressed by node, `Bug1a` re-cut by field (the swarm
      re-reads frames every frame, so a field swap *is* the rebind),
      `Spray` re-decoded by hand, `Leofont` re-typeset as derived
      text. Game state untouched throughout. Checked 2026-10-10.
- [x] **Shaders** — landed as `shaders::Live`: every `.wgsl` watched
      (ten stats a frame, debug builds on filesystem targets), a
      touched file re-assembles the whole table from disk, naga —
      the same two stages wgpu runs — vets the batch, and a passing
      batch rebuilds the pipelines by the same road a surface-format
      change always took. Proven live: a broken `sprite.wgsl` was
      refused (*the old shaders stand*) with the game rendering on;
      the fix compiled three seconds later (*the pipelines rebuild
      next frame*) without a restart. The seam between watched and
      compiled is itself a test: disk assembles exactly what
      `include_str!` embedded. Checked 2026-10-10.
- [ ] **The guard** — reader landed 2026-10-10, and it followed the
      codec into the guest: `map::tileset_names` in sprite_util
      (first-spoken order, none twice, cuts deliberately out — the
      name is the cargo question), with its desk face
      `waiting_names` (identity by name *and* cut, as dressing
      decides) and `open_map` naming who waits instead of counting.
      The app-side test — *the world ships every tileset it names* —
      waits for the first app that ships a map; the half rung keeps
      the box open until `include_bytes!` can die at `cargo test`
      somewhere real.
- [ ] **The telling** — the told half landed 2026-10-10:
      `frost::Teller` — announce card in `target/frost/live/` (dies
      with the app, and the folder self-heals crashed authors *and*
      their orphaned documents by pid liveness), a `rev`-counted live
      file in the house RON dialect, published by value-key dirty
      check at frame end. immortal tells `told, config, watches,
      ships, beds` — and proved it live: save the pace file, the
      being re-told as rev 2 with the new pace three seconds later;
      revert, rev 3; in between, an idle garden wrote nothing, ever.
      One refinement over the paragraph's proposal: the key is
      *derived* (the discrete facts the snapshot holds), so no
      mutation site can forget to mark it. The rung's other half —
      a tool attaching and reading — awaits sprite_util's reader;
      the box stays open until a tool has read a being.
- [ ] **A demo to prove it** — `immortal` (or a small new example) runs
      from loose names in debug, ships as one embedded binary, and
      survives a live tileset edit mid-run.

**Code hot reload is deliberately off the scoreboard.** The dylib pattern,
if it ever comes, is a documented app pattern, never a frost API —
Bevy's book chapter stands as the reason, recorded above.
