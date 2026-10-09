# Tools live beside the engine, not inside `examples/`

> **Status.** Decided, not yet done. `sprite_util` is a tool: it edits the
> author's files, it has no interest in demonstrating an API, and it is where
> work happens daily. It currently lives at
> [`examples/sprite_util.rs`](../examples/sprite_util.rs), alongside genuine
> examples, because that is the only place Cargo has for a second program in a
> package. That is an accident of layout, and it is starting to cost: the file is
> around seven thousand lines and holds both the engine's sidecar vocabulary and
> the tool's own drag-handle heuristics in one undifferentiated mass. Nothing
> here has been implemented; what follows is the agreed direction and the first
> step that can be taken without a design decision.

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
2. Its unit tests get *better*, not different. Examples are not tested by plain
   `cargo test` — Cargo leaves `test` off for that target kind — so the tool's
   sixty-six tests run only when someone asks for `cargo test --example
   sprite_util`, which is why the verification loop in `ARCHITECTURE.md` names
   it. A binary target is tested by default, so the same tests start running in
   the everyday command with no edits.
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
