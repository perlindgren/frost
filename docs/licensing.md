# Licensing

A `license` field is a claim about the files inside one package. It says nothing
about a directory, a repository, or a neighbour — which is the whole reason this
file exists, because the arrangement before it was a repo where nothing declared
anything and everything looked like it might be free.

Measured, not felt: `cargo package --list -p frost` on the manifest as it stood
listed **372 files**, of which **235 were `assets/` and `examples/`**, including
`.DS_Store`, every `.wav`, and four fonts that no source file mentions. An
`MIT OR Apache-2.0` field written over that would have licensed a game's art to
the world by an accident of packaging.

## What each package says

| package | terms | why |
|---|---|---|
| `frost` (engine) | `MIT OR Apache-2.0`, **defines the default** in `[workspace.package]` | the library, its shaders, and the three files its own `include_bytes!` reaches for — `include` in the manifest is what makes that the crate rather than the repo |
| `frost_shaders` | inherited: `license.workspace = true` | games borrow programs from the shelf; a lender cannot hand out what it does not hold |
| `tools/sprite_util`, `tools/noisefloor` | inherited: `license.workspace = true` | engine-side tooling; `noisefloor` is a dev-dependency of the engine's benches, so its terms cannot be stranger than the engine's |
| `antifrost` (Ember Hill) | none — all rights reserved | a game and its art; it does not ask for the workspace licence, and `publish = false` marks the silence as a decision |
| `immortal` (`examples/immortal`) | none — all rights reserved | 12,044 lines of game wearing an example's name; still packaged with the engine until it is lifted out, which is the open item below |

Inheriting is **opt-in**, which is the only reason a workspace default is safe to
state: a game added to `members` tomorrow is not licensed by a decision it never
made. `cargo metadata --no-deps` is the check, because it prints what cargo
resolved rather than what a manifest appears to say — and it prints four packages
carrying `MIT OR Apache-2.0` and `antifrost` carrying nothing.

They stay separate packages rather than modules of `frost` for reasons already in
their own manifests: `sprite_util` brings `clap`, `env_logger`, `rfd` and `image`,
which "must not ride into every downstream `frost` tree", and `noisefloor` depends
on nothing *deliberately* — "an instrument that borrows its subject's dependencies
measures those". Membership is not the opposite of being a package; it is how
several packages are built together.

The invariant holding the lot together is one sentence: **the arrows point at
the engine.** An application may depend on permissive code and stay proprietary;
the engine must never depend on a restrictively licensed member, because linkage
and `include_bytes!` do not care what the directories believe.

## The one trap worth remembering

Cargo ships files named by `include_bytes!` **even when an `include` list would
exclude them**. So naming the font in the list is not what puts it in the crate.
Naming `assets/fonts/OFL.txt` is what keeps it there: Fira Code is used under the
SIL Open Font License, which requires the licence text to accompany the font, and
*no source file mentions that text* — so the first `include` list written without
it would strip the file that makes the redistribution lawful, silently, with
nothing failing anywhere to say so. `assets/fonts/README.txt` is named for the
same reason: it is the provenance, and provenance is referenced by nobody.

Verified rather than assumed: `src/backend/diagnostics.rs` embeds both
`FiraCode-VariableFont_wght.ttf` and `Leofont-Regular.ttf`, and `src/ron.rs`
embeds `assets/ron/garden.ron`. Those three files belong to the crate forever,
whatever the list says.

And the outcome, from `cargo package --list -p frost` with the list in place:
**93 files** — 69 under `src/`, 11 shaders, both licence texts, the two embedded
fonts, `garden.ron`, `OFL.txt`, its provenance README, and cargo's own three
metadata files. **Zero** sprites, sounds, scenes or maps; **zero** `.DS_Store`.
The engine's crate is the engine.

## What the published crate cannot do, and why that is the deal

`include` omits `examples/`, while the manifest still names 41 of them. Tested in
a scratch package rather than argued from documentation, because the answer is
not what the intuition says:

| command | result |
|---|---|
| `cargo build` — what a consumer does with a dependency | ok |
| `cargo package`, verification on — what `cargo publish` runs | ok |
| `cargo build --all-targets`, and anything that compiles examples, including `cargo test` | fails: `can't find example … at path …` |

Packaging passes because verification builds the library alone; the manifest
parse tolerates a named example whose file is absent and only target resolution
objects. So the crate publishes cleanly and consumes cleanly, and an unpacked
tarball cannot build its demos or run its own test suite — a git checkout does
both, which is where examples are meant to be run anyway.

The limitation exists because shipping `/examples` means shipping what they
`include_bytes!`, and that is the game's art — the exact thing this file exists
to keep out. Give the engine's examples their own fixture art (the first of the
three endings below) and `/examples` can join the list without costing anything.

## What is still open, named as such

- **The engine's examples embed the game's art.** 11 files under `assets/` are
  referenced by `immortal` *and* by engine-side code — `tomato.png`, `grass.png`,
  `plant1..3.png`, `Waspeye1.png`, `Worm1_crop.png` among them — so a demo like
  `examples/shapes/tomato.rs` compiles a game's sprite into the MIT package. Three
  honest endings: give the engine's examples their own fixture art (cleanest),
  license that art MIT/Apache as well, or move those examples out with the game.
  Undecided as of this writing; the `include` list keeps the *published crate*
  honest meanwhile, which is why it was written first.
- **39 asset files belong to `immortal` alone** and should move to an
  `immortal/assets/` when it is lifted into a package — `antifrost/assets/`
  already holds 30 of its own, so the shape exists.
- **76 asset files are referenced by nothing** and still sit in the tree. Out of
  the crate now, but still repo content whose owner a reader cannot tell.
- **Three fonts and every sound have no provenance note.** `assets/fonts/OFL.txt`
  and `assets/fonts/README.txt` cover Fira Code properly — that instinct was
  already here. Leofont, JameGem08 and Barrfont say nothing about who made them,
  and to a distributor "mine" and "unknown" look identical. One line each in a
  `CREDITS` file is the whole obligation. Same for the `.wav` files.
- **Inbound contributions.** The dual licence is clean to grant today: first
  commit 2026-09-11, one author. That stops being true the moment anyone else
  lands a patch, and there is no contributor agreement — so the day a stranger's
  commit is accepted, this file needs a line about what terms contributions
  arrive under, or the ability to relicense later depends on finding them.
- **Before any real `cargo publish`.** Packaging is proven; uploading is not, and
  cargo already says part of it: `manifest has no description, documentation,
  homepage or repository`. `noisefloor` is also a path dev-dependency of the
  engine, which a publish will want resolved somehow. Neither has been exercised,
  and `--list` proves what would ship, not that crates.io would accept it.

## Where nice-tuner fits

If it joins the workspace it brings its own manifest, and its terms are its own
business — MIT/Apache today, changeable later in one file without touching the
engine. The arrangement to avoid is the one it is in now: a vendored copy of the
engine sitting inside a crate that declares `MIT OR Apache-2.0`, while the engine
declared nothing at all. That is how a wrong licence gets shipped — not by
decision, but by adjacency.
