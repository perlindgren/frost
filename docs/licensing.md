# Licensing

Cargo's `license` field records declared package licence metadata; it is not
proof of ownership, a substitute for applicable licence notices, or a way to
override third-party terms. Omitting it leaves that metadata unspecified.
`publish = false` prevents Cargo registry publication, not redistribution by law.

An earlier `cargo package --list -p frost` listed 372 files, including 235 under
`assets/` and `examples/`. Publishing unrelated game material with engine licence
metadata could create scope ambiguity and could distribute assets without the
necessary permissions. Packaging alone cannot grant rights the distributor
does not hold. The package inclusion rules reduce that risk; they do not resolve
asset ownership or permissions.

## What each package says

| package | terms | why |
|---|---|---|
| `frost` (engine) | `MIT OR Apache-2.0`, **defines the opt-in metadata** in `[workspace.package]` | the library, its shaders, and the three files its own `include_bytes!` reaches for — `include` in the manifest is what makes that the crate rather than the repo |
| `frost_shaders` | inherited: `license.workspace = true` | games borrow programs from the shelf; a lender cannot hand out what it does not hold |
| `tools/sprite_util`, `tools/noisefloor` | inherited: `license.workspace = true` | engine-side tooling; `noisefloor` is a dev-dependency of the engine's benches, its declared terms are MIT OR Apache-2.0 |
| `nice-tuner`, `nice-tuner/crates/pitch`, `nice-tuner/xtask` | inherited: `license.workspace = true` | Current metadata declares MIT OR Apache-2.0. Earlier metadata differed; changes to it do not withdraw valid earlier grants. Third-party components retain their own terms. |
| `antifrost` (Ember Hill) | [Rights reservation](../antifrost/LICENCE.txt) | Covers only the rights described in the notice; preserves third-party terms, earlier grants and statutory rights. No inherited Cargo licence metadata; registry publication disabled. |
| `immortal` | [Rights reservation](../immortal/LICENCE.txt) | Same qualifications as Antifrost. Its move from an engine example to a separate package does not revoke any valid earlier permissions. |

Cargo workspace licence inheritance is opt-in. `cargo metadata --no-deps`
reports resolved metadata, not the full legal status of each file. The games'
separate notices state the intended reservation; applicable component licences,
previous valid permissions and law remain controlling for their respective scope.

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
- **Asset provenance remains incomplete.** Fira Code has accompanying OFL terms
  and provenance information. The ledger lacks sufficient provenance records for
  Leofont, JameGem08, Barrfont and sound files. Identify the relevant creators,
  rights holders, permissions and applicable conditions before distributing
  affected assets. Attribution alone does not establish permission or necessarily
  satisfy a licence. Missing records do not establish ownership in either direction.
- **Inbound contributions and ownership.** Git author names and commit history
  are evidence of development activity, not proof of copyright ownership or
  authority to license every contribution. Applicable contribution terms and any
  necessary permissions should be checked before accepting or relicensing work.
  A separate contributor agreement is not automatically required, but should not
  be assumed to exist; existing valid grants must be respected.
- **Before any real `cargo publish`.** Packaging is proven; uploading is not, and
  cargo already says part of it: `manifest has no description, documentation,
  homepage or repository`. `noisefloor` is also a path dev-dependency of the
  engine, which a publish will want resolved somehow. Neither has been exercised,
  and `--list` proves what would ship, not that crates.io would accept it.

## Where nice-tuner fits

Nice Tuner declares MIT OR Apache-2.0 metadata by inheriting the workspace
field. Its dependencies and embedded assets retain their applicable terms;
metadata inheritance is not evidence of their ownership or permission scope.
The plugin depends on the engine by path rather than through a vendored copy.

Fira Code is separately licensed under the SIL Open Font License. Its licence
and copyright notice must accompany redistribution as required by those terms.
Game sprites, maps and audio are outside the intended plugin release contents;
their rights and provenance must be assessed individually rather than described
collectively as owned by this project. The release packaging below includes the
font notices; that does not resolve provenance gaps elsewhere in the repository.

The family's second embedded font, the Fluent System Icons face its panels draw
their glyphs from, belongs to `nice-common`: that package's `assets/fonts/` holds
the font, its provenance README and its MIT notice, and
`nice_common::fonts::icon_font()` is the one place the bytes are embedded. It used
to sit in `nice-tuner/assets/fonts/` while seven other crates named that path,
which no manifest could record as a dependency — an asset seven editors share is
not one plugin's property. The bundler ships the MIT text into every bundle the
workspace builds, for the same reason as the OFL text: all eight editors embed the
font, so all their distributions carry the permission.

## Plugin and standalone release notices

`cargo xtask bundle` keeps the small project MIT/Apache, Fira Code OFL and Fluent
System Icons MIT texts in directory bundles' `Contents/Resources` (or beside
single-file CLAP exports), together with inspiration and non-affiliation notes.
Nice NAM also carries its dependency's upstream NOTICE. The original copy in
`nice-nam/NOTICE.txt` matches both currently pinned engine versions.

The large third-party collection is **shared per download**, not embedded in
every executable or repeated inside every plugin. It consists of:

- `THIRD-PARTY-NOTICES.txt`: original legal texts and selected standard terms,
  deduplicating identical content while preserving attribution/source pointers.
- `DEPENDENCIES.tsv`: exact versions, declared/selected terms, target, source
  download URLs, repositories and authors (not inferred copyright holders).
- `MPL-SOURCES.txt`: exact-version source locations for the unmodified MPL
  components. Source code is not bundled; recipients receive a route to the
  corresponding covered source under MPL-2.0.

Build a complete download containing the selected plugin formats, standalone
applications and rack, with one shared notice set:

```sh
cargo xtask package-release -p nice-tuner -p nice-nam -p nice-dist -p nice-phase \
  -p nice-clone -p nice-echo -p nice-ecomplex -p nice-wah -p nice-pi-face \
  -p nice-rack --release --target aarch64-apple-darwin --locked
```

Use the required target triple explicitly (also for native builds), so a local
Cargo configuration cannot cause a different dependency inventory. Windows uses
`x86_64-pc-windows-msvc`; the Linux NAM engine requires the documented per-command
`RUSTFLAGS="-Ctarget-cpu=x86-64-v3"` with `x86_64-unknown-linux-gnu`. Do not add that
flag globally. Rust targets, native system libraries and macOS codesigning tools
must be available on the build host. This is packaging, not cross-linking support.

Output is under the actual Cargo target directory:
`downloads/frost-nice-<target>/` and `downloads/frost-nice-<target>.tar.gz`.
Only the selected current builds enter it. Small notices remain inside directory
plugins; standalones and single-file plugins share the top-level texts. NAM's
upstream notice is also present at the download root when NAM is selected.

The archive is read back and required texts compared byte for byte. Settings,
models, cabinet WAVs, artwork, source files and OS droppings are rejected by the
release asset policy. The Python checker additionally verifies archive layout,
MPL source pointers, one shared notice collection and hashes/origins of the
reviewed legal texts:

```sh
python nice-tuner/xtask/scripts/check-release-licenses.py \
  target/downloads/frost-nice-aarch64-apple-darwin.tar.gz
```

`cargo xtask package-standalone -p nice-tuner -p nice-nam -p nice-rack --release`
still stages per-application executable directories with small notices under
`packaged/`; the large collection is shared at `packaged/`, not repeated in each
application. These staging directories are not independent complete downloads:
use `package-release` with one selected package if offering separate downloads.
Likewise direct `bundle` leaves its shared collection beside the bundle set.

For notice validation without compilation use `cargo xtask release-notices -p
nice-nam --features nice-nam/standalone --target <triple> --locked`. The native
default is accepted for this inspection command; configured cross-builds must
name the target explicitly. Universal bundles collect both macOS architectures.
Build different targets sequentially or use separate target directories.

`release-licenses.yml` checks legal-text provenance, failure-case unit tests and
metadata inventories for macOS, Windows and Linux. The final downloadable archive
is checked by the packaging command itself; CI also needs to invoke the Python
checker on archives it publishes. Publishing/tag-triggered binary workflows are
not installed by these notice checks.

See [the provenance collection](third-party-licenses/README.md) for scope and
upstream omissions. These checks do not establish title to third-party material,
resolve SDK/patent/trademark questions, or replace review of any added native
redistributable. Mozilla's [MPL FAQ question 8](https://www.mozilla.org/en-US/MPL/2.0/FAQ/)
explains the source-location duty for executables built from unchanged code.
