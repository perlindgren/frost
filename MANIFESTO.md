# Amendments on the making of engines

The station's founding stance, in the form it argues in: numbered articles,
each separable, each testable, each repealable by evidence. `COMPARISON.md`
says where frost stands against the field; this document says *why it
stands there* — the fusion this station was built on, naming the trade
the author came from: a career in hard real-time systems, computer
architecture, and compiler technology, arriving in an industry that
guesses, with the intention of importing rigor instead of inheriting
vibes. Game design itself — fun, feel, the reason any of this matters —
is the journey's other half, and Article XI keeps that honest.

An amendment is alive if the scoreboard has a rung for it. An amendment
whose rung is checked and found false is amended or repealed, in writing,
in this file. That is the whole religion.

## I — The frame is a deadline

A game at 60 Hz is a recurrent task set with implicit deadlines, and
smoothness is a *distribution*, not a number. Average fps is a vanity
metric; median, p99 and max are the product, and the clock that owns the
schedule (vsync, ProMotion's rebase, a compositor) is the first suspect
for every jitter story. Missed frames get a declared policy — shed load
by admission control, degrade by design — because an engine without a
miss policy is praying.

*Scoreboard: the tail-latency habits behind every measurement in the
verification culture; the jitter investigations stand as precedent.*

## II — Time-triggered honesty

`process()` is a time-triggered loop: every frame is a pure function of
state sampled on a clock. The scene tree is a frame artifact, not a
stored truth — which forecloses by architecture the entire genus of
stale-event bugs that event-graph engines pay for forever (dirty
flags forgotten, signals ordered by accident, an editor's view and the
runtime's view drifting apart). The cost is honest: recomputation buys
freshness with CPU, every frame, by choice. Thirty years of
time-triggered-versus-event-triggered argument arrived at a game engine
and the argument was settled the same way it always was — by naming the
trade instead of inheriting it.

## III — Compilation is the architecture

An engine-as-library is not a taste claim, it is a compilation claim:
static linking admits whole-program optimization — LTO, inlining,
monomorphization as free specialization — across the engine/game
boundary, and every runtime standing in that boundary (a scripting VM,
a reflection registry, a scene interpreter) is an optimization barrier
no embedded engineer would tolerate. When the library claim is only
about the loop while the mass is a framework, the trench coat shows;
frost's version is that the engine compiles *into* the game, ships as
one binary (`include_bytes!` and all), and the game code the reader
sees is the whole program. Bevy's dylib chapter and Unity's IL2CPP wall
stand as the field's own evidence that the runtime is a tax, paid twice
— once for reload, once forever.

## IV — Determinism is testability

Seeded streams, state that a save file carries across processes
(`Rng::state()`/`set_state()`), fixed-point discipline where it matters,
reproducible runs as a precondition for trust. A heisenbug is a design
finding, not a weather condition. Lockstep and replays are what
determinism looks like when a network forces the confession; the station
prefers to hold the property before it is rented out.

## V — Truth is the program; tools are guests

frost inverts the Godot contract: no file holds the application, so no
tool may *extract* state — it may only be *told*. Documents (saves,
maps, gameplay RON) are the truth of a sleeping app; transition-driven
snapshots (`state_of_affairs`, rev-counted, flushed at frame end — an
autosave with manners) are the truth of a running one; tools write
documents, never memories; the watched-file mechanism that powers hot
reload is the only courier, in both directions. Bidirectional scene
editing — the editor as second author of a book it cannot read — is
refused on the record.

## VI — Names before paths

An asset's identity is its name — the stem — plus the cut made into it
(the grid), never a directory path. The same name resolves in the repo,
beside the loose binary, and inside a shipped single binary, so the
development loop and the shipping model share one seam instead of
negotiating two. `("basic_tiles", 2, 8)` is the whole of an asset's
identity in every world; a forgotten `include_bytes!` dies at
`cargo test`, not in the field.

## VII — Data is the hot tier

The reload ladder climbs from its cheapest rung: gameplay values living
in RON, re-parsed on change, the running app rebuilt from the new root
— iteration at editing speed, which is where *design* iteration lives,
which is the point. Media, maps, shaders follow by cost. Code reload
stands last and outside: the dylib reloader is a documented app
pattern, never an engine API, because the ABI tax would be paid by
everybody forever for one person's convenience, and Bevy's own manual
says so.

## VIII — Measurement before mechanism; findings before plans

Nothing enters the architecture that was not first seen in a number
(the jitter forensics, `ft med/p99/max`, the 0.7 ms text-layout finding
carried as debt, not rumor), and nothing enters the *plan* that was not
first recorded as a finding — the labels are in the documents on
purpose: *this paragraph is the finding, not the plan*. Claims without
a scoreboard rung are marketing; the scoreboard's boxes stay unchecked
until code and tests say otherwise.

## IX — Smallness is a surface tax

Every dependency is runtime, every public symbol is contract, every
layer of indirection is a reader's cost and a cache line's surprise.
The dependency tree is defended like a hard real-time boundary (arboard's wasm
breakage is the standing cautionary tale); the API is grown the way a
safe language grows surface: rarely, named out loud, with the
consequence stated first. A thousand-feature engine is a thousand
owners' decisions; frost's features are the author's, held to the
same account.

## X — The tail is the product

Everything in I and VIII compressed to a consumer-facing sentence: the
player never felt the average frame. Budgets, headroom, and the
declared degradation of a bad frame are the user-visible engine; the
rest is infrastructure talking.

## XI — The humility clause

None of this makes a game. Fun is not derivable from deadline
discipline; feel lives in the immortal TODO, in art, timing, and
judgment the station is still growing toward. Rigor is what lets the
designer iterate at the speed of thought — it has never once told
anyone *what* to build, and it never will. The engine is the instrument;
the journey is learning to play it, and the demos are the sheet music.

---

*Filed by N65 Game Research Station, October 2026 — as much a learning
journal as a constitution, and prouder of the first part than the
second. Cross-references: `ARCHITECTURE.md` (Direction: the findings
these amendments stand on), `COMPARISON.md` (the field, and the
scoreboard each article answers to).*
