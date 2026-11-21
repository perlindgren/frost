# Structural graph editing

The rack document is an ordered series-parallel expression, independent of pixel
coordinates and column spans:

- `Plugin(id)` is one existing CLAP instance.
- `Series(children)` passes each child's output to the next child.
- `Parallel(children)` feeds the same input to every child, in top-to-bottom
  order, and sums their outputs at a local join.
- `Empty` is the rack with no plugins (input passes to output).

A plugin ID occurs exactly once. Editing never clones an instance, changes its ID,
resets its parameters, or silently distributes a shared processor across paths.
Audio reconfiguration still follows the existing stop/snapshot/reuse protocol.

## Operations and drag events

| Intent | Operation | Drop zone |
|---|---|---|
| Insert in series before a plugin | `Before(target)` | Left of its card |
| Insert in series after a plugin | `After(target)` | Right of its card, before its existing outgoing connection |
| Create a local parallel branch above a plugin | `Above(target)` | Top of / directly above its card; above its outgoing wire |
| Create a local parallel branch below a plugin | `Below(target)` | Bottom of / directly below its card; below its outgoing wire |
| Insert before a complete fork | `BeforeFork(scope)` | Wire immediately before that fork's split |
| Insert after a complete fork | `AfterFork(scope)` | Wire immediately after that fork's join |
| Add at the end | `Append(id)` | A new CLAP file without a local drop target |
| Remove an instance | `Remove(id)` | Card's × |

Moving is **detach, insert, normalize**, as one transaction:

1. Validate unique IDs and that the target exists. Capture the exact group for a
   wire target before detaching anything. Every visible fork carries its own
   group scope; the first card's identity does not identify a nested fork.
2. Remove the moving plugin from its old position, closing its serial gap. An
   emptied branch disappears rather than becoming an implicit dry-signal branch.
3. Insert around the target plugin (or outside the targeted fork). Every subtree
   not involved in the edit retains its wiring and branch order.
4. Remove empty groups, unwrap one-child groups, flatten adjacent serial groups,
   and flatten directly nested parallel groups in order. Thus redundant splits
   and joins fold into a single multiway junction whenever equivalent.
5. Validate the resulting expression and its audio widths before adopting it.
   Failed edits leave the running document and plugin states intact.

Normalization does not distribute `Series(Parallel(...), Parallel(...))` into
independent paths: doing that would duplicate plugin instances and change sound.
Moving a plugin beside another affects that one plugin, not its whole former
branch. Moving within a branch, between branches, out of a fork, or across stages
uses precisely the same operation.

Internal drags and OS file drops use the same hit-test and operations. A new file
is given a new ID, appended, then moved to its selected target. Card faces take
precedence over nearby wires. The incoming/outgoing fork wire carries its exact
scope and shows a normal vertical insertion bar; the circles themselves are not
drop targets. A
release on the dragged card or in distant empty space cancels an internal move.
Escape and loss of pointer likewise cancel. A release without a drag still opens
the editor. Numeric controls/meters retain priority over graph dragging.

## Agreed example sequence

`S(...)` means series, `P(...)` means ordered parallel branches. T = Tuner,
D = Dist, E = Echo, C = Ecomplex, W = Wah.

| Action | Result |
|---|---|
| Start | `S(T, P(S(D,C), E))` |
| Echo between Dist and Ecomplex | `S(T,D,E,C)` |
| Dist below Ecomplex | `S(T,E,P(C,D))` |
| Tuner left of Dist | `S(E,P(C,S(T,D)))` |
| Dist right of Ecomplex, before join | `S(E,P(S(C,D),T))` |
| Add Wah before Tuner | `S(E,P(S(C,D),S(W,T)))` |
| Wah below Dist | `S(E,P(S(C,P(D,W)),T))` |
| Echo above Ecomplex | `P(S(P(E,C),P(D,W)),T)` |
| Dist right of Echo | `P(S(P(S(E,D),C),W),T)` |
| Wah right of Dist | `P(S(E,D,W),C,T)` |

Both the expression tests and a replay through GUI hit-testing cover this
sequence. Additional tests cover all move pairs/directions in small nested racks,
branch isolation during DSP lowering, normalization, invalid IDs/targets,
adjacent/self moves, removals, exact fork scopes, settings/geometry round trips,
and non-overlapping nested layout. The opt-in native CLAP recovery test exercises
parallel/serial moves and removal using the repository's Tuner export.

## Execution, display, and persistence

The audio compiler lowers the expression directly into plugin, mixer and explicit
channel-map nodes. Each parallel group owns its own mixer: an unrelated branch
at the same visual column is never joined there. The deterministic execution
priority follows branches top-to-bottom, while respecting dependencies. Buffers,
latency compensation, timing observations and native lifetime management stay in
the existing engine/CLAP adapter.

The structural canvas derives slots, cables, split/join rings and drop scopes
from that same expression. It does not reconstruct connections from coordinates.
Scroll extents account for chain length, not merely the number of stages.

RON saves the expression in the first plugin entry's optional `routing` record.
All live cards retain the shared expression, so deleting its first instance does
not lose routing. Plain string tags (`kind: "parallel"`, `kind: "series"`,
`kind: "plugin"` with `id`) survive the shared settings code's RON value and
window-geometry updates. Plugin states, paths, trims, mute/pause and editor
geometry retain their existing format. No JSON or dependency changes are needed.

Old stage/branch/span files are imported when their spans have an unambiguous
structured interpretation. Their legacy metadata remains a compatibility
projection, not the authority after migration. Crossing spans are refused for
structural editing rather than guessed or silently rewired. The legacy reader
and renderer remain available for such old files, and autosave does not panic.
The restored three-branch rack is an unambiguous import.

New nested documents should be edited with this version of Nice Rack; an older
binary understands only the compatibility lane projection and cannot preserve
all nested routing semantics.

## Equivalent drop slots

The UI caches a semantic drop table for each routing expression and dragged ID.
Candidates are compared by their complete normalized result **after detachment**.
Each distinct ordered graph has one canonical target and one insertion marker:

- After A / before its serial successor B share the cable midpoint.
- Below A / above adjacent parallel sibling B share the gap midpoint when both
  actually produce the same local graph. Forking a member of a longer serial
  branch is not confused with adding an outer parallel branch.
- A serial endpoint next to a fork shares the fork's adjacent wire target.
- After a join / before the next split share one serial slot.
- Card-face and wire above/below gestures use the same Above/Below operation;
  the old Over/Under variants have been removed.
- Self/adjacent moves that reproduce the unchanged document have no drop marker.

Branch order is part of equivalence: top-to-bottom execution is preserved.
Internal moves and external file drops use this same table. Pointer movement and
scrolling reuse it; the potentially more expensive graph comparisons run only
when the routing or dragged instance changes.
