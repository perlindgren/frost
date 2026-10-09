# Tools live beside the engine, not inside `examples/`

> **Status.** Decided, and steps 1–2 are now done — see the **Done** notes
> below the steps. `sprite_util` is a tool: it edits the
> author's files, it has no interest in demonstrating an API, and it is where
> work happens daily. It used to live at
> `examples/sprite_util.rs`, alongside genuine
> examples, because that is the only place Cargo has for a second program in a
> package. That was an accident of layout, and it was starting to cost: the file is
> around seven thousand lines and holds both the engine's sidecar vocabulary and
> the tool's own drag-handle heuristics in one undifferentiated mass. It now
> lives at
> [`tools/sprite_util/main.rs`](../tools/sprite_util/main.rs), its own package
> in a Cargo workspace, and the layout half of this document is history; what
> remains open is step 3 — the splits inside the tool — and the second tool
> that will decide the fate of the shared `ron_view` row view.

## The principle

Two questions, applied to any piece of code in this repository, and they have
different answers.

*Is this the engine's business?* Then it goes in `frost` — which is a
commitment, not a relocation, because it becomes public API and needs the docs
and tests that implies. The RON-subset parser moved for exactly this reason
([`src/ron.rs`](../src/ron.rs)): the engine must read the sidecar, so the parser
is not example code that the engine borrows. The same argument covers the
authored-shape vocabulary ([`docs/authored-shapes.md`](authored-shapes.md)), and
probably the tile-map model, which today exists only as fields inside a tool and
in that tool's undo snapshots, with no file format anywhere.

*Is this the tool's business?* Then it stays in the tool and out of the engine,
however tempting the move. Drawing selection handles, the `status:` line, undo
history, keyboard dispatch: none of it is a fact about the engine, and pulling
it in because it is well written would make the engine answer to a GUI.

The awkward cases are the assumptions the two share. The clearest one is
`spot_coords`: the tool treats *any* struct with exactly two numeric atoms as a
draggable position handle, and that heuristic has been cited as the reason the
sidecar's `atlas` key is an array and as the reason authored positions must be
arrays too. The principle resolves it by splitting the assumption rather than
moving it. Positions are arrays in the format because a position is data and the
engine must not mistake it for a record — that is the engine's. "Two numeric
fields means the author wants to drag it" is a guess about intent that only the
tool ever made, so the tool stops guessing: when it learns to edit shapes, a
handle attaches to a key the tool knows, not to a coincidence of arity.

## Where it goes

A `tools/` directory holding the tools and their shared helpers, declared as
explicit `[[bin]]` targets with `path` pointing into it. Cargo has no notion of
a tool, but it has targets, and a binary target is the honest description of
what these programs are.

The first step is the boring one and it needs no decisions:

1. Move the file to `tools/sprite_util/main.rs`, declare `[[bin]] name =
   "sprite_util"` with that path, and drop the `example` entry. Same package, so
   nothing else moves: no workspace, no new crate, no asset paths to retune.
   **Done:** the file moved to
   [`tools/sprite_util/main.rs`](../tools/sprite_util/main.rs) and builds as
   the binary `sprite_util` — but by the *second* option of the decision
   below, not the first: the tool is its own package in a workspace
   (`clap`, `env_logger`, `rfd`, `image`, `log` live in its manifest, and
   `frost`'s dependency tree is byte-identical through it), because
   promoting tool-only dependencies into the library was never acceptable.
   So "no asset paths to retune" did not hold: the runtime font path is
   now built from the repository root by one helper, guarded by a test
   (`the_hud_font_is_where_the_asset_helper_points`), and the plant1
   fixture's `include_str!` gained a `../`. There was no `example` entry
   to drop — the target was auto-discovered. And the sixty-six tests are
   sixty-eight — the move carried 68 tests verbatim, plus a 69th, the
   asset-path guard.
   (The "byte-identical tree" claim later stopped holding on its own: an
   `arboard` for the `text_input` example and an unused `rfd` crept into
   `frost`'s `[dependencies]`, and once a stale lockfile stopped hiding
   them, `arboard` — which does not compile for wasm32 — broke the wasm
   check of the library. Both are dev-dependencies now, which is where
   example-only dependencies live and what this step's whole argument
   insists on.)
2. Its unit tests get *better*, not different. Examples are not tested by plain
   `cargo test` — Cargo leaves `test` off for that target kind — so the tool's
   sixty-six tests run only when someone asks for `cargo test --example
   sprite_util`, which is why the verification loop in `ARCHITECTURE.md` names
   it. A binary target is tested by default, so the same tests start running in
   the everyday command with no edits.
   **Done, with one edit the step did not predict:** "no edits" assumed the
   binary shared the root package. In its own package it is not a *default*
   member — a workspace with a root package defaults to the root package
   alone — so the root's `[workspace]` names `default-members = [".",
   "tools/sprite_util"]` and plain `cargo test` really does run them.
3. Then, and only then, split the file along the seams that measurement finds,
   because a module boundary inside a tool is a normal edit while a module
   boundary across the engine/tool line is a decision about API.

**The step is not as boring as it looks, and this is the decision inside it.**
The tool parses its arguments with `clap` and installs `env_logger`, and both are
`[dev-dependencies]` — a binary target does not see dev-dependencies, so the
move will not compile until either both are promoted into the crate's real
dependencies, which puts them in every downstream tree to serve one editor, or
the tools become their own package, which is the end state this document is
pointing at anyway and brings the asset-path question with it early. Choosing
the second is honest about what `tools/` means; choosing the first buys the
layout now and leaves the second option open.

Two further things to check before step 1 rather than after. Asset paths are
pinned to the crate root via `CARGO_MANIFEST_DIR`, which does not change inside
the same package but would silently re-point the day a tool becomes its own
package — the font is loaded that way, and a silent re-point is worse than the
two compile-time path fixes the move needs. And the verification commands in
`ARCHITECTURE.md` name `--examples` in the gate; a target change has to update
them in the same commit, or the document starts describing a build nobody runs.

## What does not move

The `[[bin]]` step is layout. It does not make `sprite_util` engine code, does
not by itself shrink the file, and does not licence moving the tile-map model or
the sidecar editing UI into `frost` because they are now nearby. Proximity is
not an argument. Each of those crosses the engine/tool line on its own merits,
one at a time, with the surface it is committing named out loud first.

## A reason to prefer a binary over an example

An argument for this layout that has nothing to do with dependencies: `cargo
clippy --all-targets` does not compile example targets with `cfg(test)`, so the
test code inside the ~48 remaining examples has never been linted — `immortal`'s
hundred-odd tests among them. Binary test harnesses *are* linted, which is why
moving this one tool surfaced five warnings on the spot, proven by compiling a
byte-identical copy of the file both ways: zero as an example, five as a binary.

So every tool that moves into `tools/` gains lint coverage for free, and the
gap left behind in `examples/` is a backlog whose size nobody has measured. It is
left unfixed deliberately: a lint sweep across 48 demos is its own change, with
its own review, and it should not ride into a packaging commit where a broken
demonstration would be blamed on the workspace.

## Games come too, later, one at a time

The games follow the same road — `dogs_name` and `immortal` as workspace
members, and eventually their own crates. Not now, because nothing forces it and
the move is churn for its own sake. Two things worth noticing when it does
happen.

A game becoming a binary target buys the same thing the tools bought: its test
code starts being linted, because clippy compiles bin harnesses and not example
ones. That pays down the backlog above one game at a time, for free, which is a
better reason to move a game than tidiness.

But keep at least one game in `examples/`. An example is the closest thing in
this repository to what a stranger writes — it sees `frost` only through the
published surface, with no shared manifest and no path shortcuts — and once
every game is a member of the workspace, that view is gone. The games are the
author's; the example is the user's, and the two fail in different ways.
