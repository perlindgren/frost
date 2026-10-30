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
| `immortal` (`immortal/`) | none — all rights reserved, stated by the manifest's silence: a `[package]` with no `license` key, in a workspace where engine-side packages inherit `MIT OR Apache-2.0` only by asking for it | The garden game, 13,135 lines, a package of its own since 2026-10-29. Before that it was an example *target* of the `frost` package, which meant the nearest licence statement was a claim about a crate it was not, and the only record of its terms was a comment and this row — nothing a distributor could check. `publish = false`, no `include` list, and 78 art files of its own |

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

And the outcome, from `cargo package --list --allow-dirty -p frost` with the list in
place: **92 files** — 69 under `src/`, 11 shaders, 5 under `assets/` (the two
embedded fonts, `garden.ron`, `OFL.txt` and its provenance README), both licence
texts, `README.md`, and the four files cargo generates for every package:
`Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock`, `.cargo_vcs_info.json`. **Zero**
sprites, sounds, scenes or maps; **zero** `.DS_Store`. The engine's crate is the
engine.

This row read 93 until it was recounted, and the breakdown behind it summed to
neither — worth recording because both ways it went wrong are easy to repeat.
Counting needs `-p frost`: run without it in this workspace, cargo lists every
default member with paths relative to each package, so antifrost's sprites arrive
as bare `assets/sprites/…` and the engine appears to be shipping a game. And on
Windows cargo prints `\`, so any filter written with `/` matches nothing in silence
— the same trap that once reported "zero source files" here.

## What the published crate cannot do, and why that is the deal

`include` omits `examples/`, while the manifest names 42 of them explicitly and
cargo resolves 57 example targets in all. Tested in
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

- **The engine's examples still draw the game's art.** Ten files the game embeds
  are named by engine code too — `grass.png`, `tomato.png`, `tomato_fg.png`,
  `plant1..3.png`, `Waspeye1.png`, `Worm1_crop.png`, and the two fonts — plus
  `plant1.ron`, the anchor sidecar the game reads loose and `sprite_util` reads
  as a fixture. They stayed at the repository's root when the game took the rest
  of its art, so a demo like `examples/shapes/tomato.rs` still compiles a game's
  sprite into an MIT-named target. Three honest endings, unchanged: give the
  engine's examples their own fixture art (cleanest), license that art
  MIT/Apache as well, or move those examples out with the game. Undecided as of
  this writing; the `include` list keeps the *published crate* honest meanwhile,
  which is why it was written first.
- **The asset tree is partitioned by evidence, and says which files lack it.**
  78 files now live in `immortal/assets/`: 47 came with the package on
  2026-10-29 — 35 named by the game's table, 12 more because a sprite never
  leaves its `.pxo`/`.ron` sidecars — and 31 more afterward, being art the game
  stopped loading: the bug frames behind the `Bug1a/2a/3a` it ships now, a second
  tomato variety with its right-facing variants, bee eyes, the basket it redrew as
  `BasketBack`/`BasketFront`, and two sounds it re-recorded as `TomatoDrop.wav`
  and `WaterFlowSoft.wav`. None of the 31 is named by any code; what attributes
  them is the commit that drew each one and what else that commit touched, with
  `examples/immortal*` excluded from the engine side by hand — the game lived in
  `examples/`, so a query that counts `examples/` as engine measures the game
  twice, which is exactly how a first pass of this measurement reported every
  file as shared. Moving them changes no key and breaks no test, because neither
  can see them: keys are root-relative and both roots are in the resolver's chain.
- **31 asset files at the root are still named by nothing, and stay there because
  the evidence stops short.** The bird sheets and their `_crop` variants, because
  the crops arrived with engine commits while the sheets arrived with the game's,
  and a family split down the middle is worse than a family left where it is;
  `Slop.png` with its sidecar, drawn by an engine particle-system commit under a
  name that could belong to either; `Worm1.png` and `Worm2.png`, the source sheets
  behind a crop the engine's `worm.rs` draws and a crop the game ships;
  `bench.png`, alone among these with a word the engine also uses for its own
  benchmarking; and `assets/sprites/METADATA.md`, whose anchor notes name both the
  files now in the package and the ones that stayed. None of that is a claim of
  ownership in either direction, which is why none of it moved. Provenance is the
  gap below, and these are its remaining instances.
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
  Settled 2026-10-29 as a decision rather than a gap: no contributors are
  expected, so no agreement is written and the README says nothing about
  inbound terms. This bullet is the trigger to revisit, not an action item —
  the first commit from anyone else reopens it, and the cost of reopening
  grows with every patch accepted before it is written.
- **Before any real `cargo publish`.** Packaging is proven; uploading is not, and
  cargo already says part of it: `manifest has no description, documentation,
  homepage or repository`. `noisefloor` is also a path dev-dependency of the
  engine, which a publish will want resolved somehow. Neither has been exercised,
  and `--list` proves what would ship, not that crates.io would accept it.

## Where nice-tuner fits

It is a member now, and it declares its terms in one inherited line —
`license.workspace = true` — so the plugin and the engine say the same thing in
two places rather than two things in one. That inheritance is opt-in, exactly as
above: a package that does not ask declares nothing.

The arrangement this section was written against is the one it came from: a
vendored copy of the engine inside a crate declaring `MIT OR Apache-2.0`, while
the engine declared nothing at all. That is how a wrong licence gets shipped —
not by decision, but by adjacency. The copy is gone; the plugin depends on the
engine by path, so there is one copy of each file with one set of terms, and no
tarball can pick up the engine under a licence the engine never granted.

What the plugin takes out of `assets/` is one font, Fira Code, under the SIL Open
Font Licence — a licence that permits embedding on the condition that its text
travels with the font. The engine's sprites, maps and audio stay all rights
reserved and do not appear in the plugin or its bundles; the bundler does not yet
carry the OFL text at all, which is recorded against `nice-tuner/src/gui.rs`
rather than assumed.
