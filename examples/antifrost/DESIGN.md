# Ember Hill — Game Design Spec (v0.2)

*Jam game, theme: **autumn** + **to keep warm**. Engine: FROST. Side-view
tile-grid puzzle: shepherd a trail of ants through an ant hill; each ant
falls asleep the moment it wanders onto its own bed, and a warmed bed
turns into a wall. Get the colony to bed before winter.*

Status: working spec, mechanics may be amended after playtesting.

v0.2 changed the rules that v0.1 shipped with: **lock-in is automatic**
(no key puts anyone to sleep) and **a warm bed is a blocker**. Every hill
was redesigned from scratch under the new rules.

---

## 1. Premise

Autumn is closing in. The colony needs to be asleep in its beds before the
first frost. You are the player ant **AP**. Stranded ants out in the
tunnels fall in behind you when you pass them — your **trail**. An ant
that comes to rest on **its own** bed falls asleep there instantly; the
colony warms in the order the line peels: **last collected, first warm**.
A bed with a sleeper is stone: nobody can walk over, push into, or pack
against it. Every delivery you make reshapes the map, so order is
survival, not just efficiency.

Win: every ant is asleep on its own colored bed and the player is on
their own bed **W** with an empty trail.

```
BP  BA  BB  BC     <- beds (fixed, authored)
AP  AA  AB  AC     <- winning state (player included)
```

## 2. Ant states

```
WILD ──(player moves into its cell)──► IN_TRAIL
IN_TRAIL, tail, resting/riding on own bed ──► ASLEEP   (automatic, the "lock-in")
IN_TRAIL, tail ──(leave-behind key)──► PLACED          (manual drop, anywhere)
PLACED ──(player moves into its cell)──► IN_TRAIL      (re-collected)
WILD / PLACED ──(pushed onto own bed)──► ASLEEP        (the "dunk shot")
```

Only the **tail** ant can leave the line (by lock-in or leave-behind), so
the trail is a LIFO stack: delivery order is the reverse of pickup order
unless you shuffle. Lock-in applies to the tail because peeling the tail
is the only removal that never crumples the line — an ant vanishing from
the middle would pack its follower straight into the fresh bed (this was
tried; it rejected essentially every multi-ant lock, which is not a game).

Ants are color-identified; beds carry the matching color. One occupant
per cell.

## 3. Tiles

| Tile   | Walkable | Notes                                              |
|--------|----------|----------------------------------------------------|
| ROCK   | no       | indestructible; frame, bed surrounds, puzzle walls |
| DIRT   | no       | **diggable** (see §5)                              |
| TUNNEL | yes      | the crawl space                                    |
| BED    | yes → **no** | empty bed is walkable; once its ant sleeps there it is solid for everyone and everything |

No gravity. Ants crawl tunnel walls; up/down are ordinary moves.

## 4. Movement — one rule per destination cell

The player presses a direction. Exactly one outcome, by destination cell:

| Destination             | Result                                                     |
|-------------------------|------------------------------------------------------------|
| Empty TUNNEL/BED        | move.                                                      |
| DIRT                    | **dig & move** if dig budget > 0 (§5); cell becomes TUNNEL.|
| ROCK / out of map       | blocked.                                                   |
| WILD or PLACED ant      | **collect** — ant appends to trail tail, player enters cell.|
| Own TRAIL ant           | **chain shift** — see below.                               |
| Bed with a sleeping ant | solid, blocked.                                            |

**The chain.** Trail ant *i* (0 = head, nearest you) rests on
`history[min(i + 1, len − 1)]` — your path history, one cell behind you
per rank, riding on you once the history runs out. A move onto the cell
directly behind you pops the history (a U-turn slides the packed line);
every other move extends it. After every action, any trail ant now
resting on its own bed is automatically tucked in (tail, see §2).

**Chain shift / jamming.** Because the line is your tail, walls block it
too: a move is rejected ("the line has nowhere to pack") when some rank
would come to rest on an occupied cell — including a bed you warmed two
moves ago. Design consequence: a U-turn immediately after a lock can jam
the follower into the fresh bed; the line must route around its own
warmth.

**Push (explicit verb).** **Shift+direction**: the first WILD or PLACED
ant in that direction is shoved one cell if the cell beyond is free — or
**locks in immediately** if the cell beyond is *its own bed* (the "dunk
shot"). Never your own line, never into an occupied bed.

**Leave-behind (Space).** Releases the tail ant onto the cell it is
resting on (a bunched or riding line drops nobody). The manual twin of
lock-in: same tail ant, dropped instead of asleep. This is the shuffle
valve: drop a tail, loop around, re-collect the placed ants in a new
order.

**The puzzle engine:** beds are delivered by *positioning* — an ant locks
in your wake where you walked, so pickup order, wake geometry, and
detach-and-loop shuffles decide who warms where. Since warmed beds are
walls, the same walk delivers or deadlocks depending on order. Two
corollaries of the U-turn pop that level design lives by:

1. **A dead-end cell is never a rest cell** (leaving one always pops it
   out of the history) — end-of-corridor beds are push-only.
2. **Walking back across your own warm bed is a wall**, for the trail as
   much as for the player; late locks must not strand the player or the
   line on the wrong side.

## 5. Digging

Player ant digs by walking into adjacent DIRT (one action, one budget).

- **Budget:** `digs` per level (0 on most early hills, 1–2 later). HUD
  shows what is left.
- **Permanence:** dug cells stay open for the attempt; Restart (R) resets.
- Digging **widens** routes (player-authored freedom) while the budget
  keeps it a decision. On **Soft Soil** the dig is mandatory — it is the
  only door.
- **ROCK** still frames everything so "dig a straight line" never wins.

## 6. Theme hooks

- **Autumn:** surface band with leaf litter palette; tile atlas carries
  it. And it is always falling: thirty leaf sprites (one particle batch
  per sheet, `Shape::Particles`) flutter in from just above the window,
  swaying, flipping edge-on, riding a slow shared gust; each lands at a
  random point in a ground band of window-height/10, fades into the
  litter, and re-circles up top.
- **To keep warm:** beds = warmth, sleeping = survival; the hill
  literally solidifies around the ants that are warm. Per-level **first
  frost par** (move count): once `moves` pass par, a **soft frost**
  creeps in from the screen edges and the win banner admits "the frost
  bit" — atmosphere and score flavor, never a blocker; undoing back
  under par thaws the screen. A hard frost timer (or hard move budget)
  is explicitly *out of scope for now* — puzzle games punish time
  pressure badly; revisit only if judged to need teeth.

## 7. Controls

| Key           | Action                                            |
|---------------|---------------------------------------------------|
| Arrows / WASD | move / collect / dig & move / chain shift         |
| Shift+Arrow   | push first loose ant in that direction            |
| Space         | leave the last ant of the line behind             |
| Z             | undo (full state stack)                           |
| R             | restart hill                                      |
| N / P         | next / previous hill (debug until menu)           |
| Esc or Q      | quit                                              |

Undo is mandatory: deadlocks exist by design (the hill walls itself off
when you deliver in the wrong order); "someone is already warm there"
and "the line has nowhere to pack" are the rules teaching themselves.

## 8. Level format

RON, ASCII grid rows (`levels.ron`), validated by `level::from_grid`
(pairs every bed with an ant, one of each, refuses orphan beds/ants,
requires a `W` bed and exactly one `@`).

```
# rock   , dirt   . tunnel   @ player start
W R Y G B = empty bed of that color (W = the player's own bed)
r y g b   = one wild ant of that color
```

## 9. The shipped hills (thirty-five, in teaching order)

1. **First Warm** — the push as first verb: one dunk, one step home.
   Par 5, BFS optimum.
2. **Attunement** — one straight hall; walk past the empty bed, the ant,
   and home, and r sleeps on R in your footsteps. The wake lock with no
   steps to spare. Par 5, BFS optimum.
3. **First Cold** — ring with one ant. Wake lock-in, the bed-wall, and
   the long way home; the solver also found the dunk shortcut that hides
   in it. Par 9, BFS-proven optimum.
4. **Fire Escape** — First Cold turned vertical: climb the shaft, stand
   on R, descend the other side — the trail tucks r into the corner bed.
   Par 10, BFS optimum.
5. **Cheap Dig** — the dirt cell *is* the shaft. Dig it, walk the ant
   up, and the corner wake beds it in ten moves; the honest way around
   is fourteen. Dig as temptation — the hill where you learn the budget
   has a price worth paying. Par 10, BFS optimum (the no-dig second-best,
   14, also found by search).
6. **Second Lap** — two ants pack behind you and peel off one after the
   other on a single walk: the tail-swap double lock, no verbs but
   walking. Par 16, BFS optimum.
7. **Split Paths** — start between two ants, every bed between its ant
   and home; one lock happens while you stand on the other ant's bed,
   one on the way *down* the shaft. Par 17, BFS optimum.
8. **Soft Soil** — one dirt door, one corner bed. Dig is the only way
   in; the bed warms at a corner you exit *from*, teaching that a bed
   only takes an ant from its wake. Par 18, BFS optimum.
9. **Double Errand** — two ants, two loops, two wake locks: either
   order of walks delivers, a breather before order starts killing.
   Par 20, BFS optimum.
10. **Mail Slot** — a bed at the end of a dead-end corridor is a hole in
    the wall, not a rest stop (corollary 1). Walking can *never*
    deliver it; the optimum leaves the ant at the corridor mouth and
    posts it in with two pushes. Par 15, BFS optimum.
11. **Two Slots** — corollary 1 twice: two dead-end beds, neither ever
    wake-lockable; the optimum parks each ant at a slot mouth and posts
    them with pushes. Par 29, BFS optimum.
12. **Ridgeway** — out along the floor collecting both, back along the
    ridge: the far bed catches r on the way out of the shaft, the near
    bed catches y on the way in. All walking. Par 22, BFS optimum.
13. **The Errand** — one ant, one dirt door, one far dead-end slot:
    dig, collect, ride the far shaft, drop at the mouth, dunk, and walk
    the whole ridge home. Every verb, one visit each. Par 22, BFS
    optimum.
14. **Doorway** — a bed *in* a corridor: cross the empty bed, collect,
    and wake-locking shuts it — the way home must detour around your
    own warmth. Beds are load-bearing. Par 22, BFS optimum.
15. **Two Ways Home** — two beds on one corridor; collect in the wrong
    order and the hill walls itself in your face. LIFO order is the
    lesson. Par 34, BFS optimum.
16. **Pantry Row** — two beds in a row, two ants reversed below: the
    calm rehearsal for Cellar Row. Par 34, BFS optimum.
17. **Cellar Row** — three ants, three beds; the optimum shuttles the
    first ant through the east shaft (drop, push, re-collect) before
    the ridge walk peels g, y, r in turn. Par 36, BFS optimum.
18. **Three Cold Nights** — three ants, leave-behind and dunk in the
    optimal route, not just as escapes. Par 41, BFS optimum.
19. **Full Beds** — twenty open cells, thirty-one moves, and the
    solver proves nothing shorter exists. The middle corridor's beds
    sit in the wrong order for any single walk, so you push y east into
    Y *while walking west* (the drop parks it exactly one shove from
    home), stash r under the hall, and let the long west walk peel g
    and r off in your wake, one step apart. The wrong-direction shove
    is the lesson. Par 31, BFS optimum.
20. **Ring Toss** — a ring whose beds all live on the return arc. You
    grab r at once, then discover a lone trailer won't settle while
    you walk: it must be parked beside its bed while you run the whole
    ring the other way, come back for it, and ride the last sweep into
    W. Two half-laps where a naive lap promised to do. Par 33, BFS
    optimum.
21. **Thimble** — the tiny thimble of a hill: a dead-end slot in the
    top wall, two wake beds in the middle, three ants hiding below.
    r locks by a two-step rewind of the wake, y peels off on a shaft
    climb, and g must be dropped at the foot of the slot and posted up
    through the hole, shove by shove. Every verb in nine columns.
    Par 37, BFS optimum.
22. **Kindling** — three stacked corridors and two shafts. Take y and g
    on the top sweep, dive to the middle floor and take r in stride,
    leave it parked mid-corridor, and drop to the floor: the west walk
    beds y on Y as you pass, the reeled-back wake beds g on its middle
    wall, and the pick-up run walks r onto R a step from home. Par 33,
    BFS optimum.
23. **Cupboard** — a ridge of bedrooms and a pocket below. Collect r
    and g mid-hall, climb, drop g on the ridge, and walk the ridge
    west: r sleeps on R under your own footsteps while you stride over
    its bed empty. Floor, pickup of y, long climb home: g rests on its
    floor bed one step behind you, y slides into Y a step before you
    seat yourself. Par 33, BFS optimum.
24. **Soot Gate** — beds in the ceiling, ants on the middle floor, and
    g's bed alone down in the dirt row. The wall bed takes g by plain
    walking; the two ceiling slots get shop work — park at the shaft
    foot, shove, step in, shove home. Two slot rhythms in one hill.
    Par 33, BFS optimum.
25. **Slow Burn** — one serpentine, one breath. y beds on the bedroom
    row as you pass its mouth; the floor shuttle collects r and g, and
    the walk *back* reels the line up the west shaft, g peeling onto G
    out of deep history; the second east pass trails r onto R exactly
    one step before you land on W. The line runs backwards into its
    beds. Par 32, BFS optimum.
26. **Pebble Gallery** — two rings sharing one middle corridor, every
    bed on the opposite ring from its ant. The optimum parks r in the
    middle, works the bottom ring collecting y and g, and comes up:
    y crosses the map into its top-row bed, g rests on the middle row,
    and r peels onto R in the last walk — the final two locks one
    move apart. Par 34, BFS optimum.
27. **Hearth Row** — three floors, three shafts, every bed in the top
    row, in an order your trail does not form. Go down, gather, come
    up with a full line, then shuffle: two drops and a shove reorder
    the trail so the ridge walk beds g at the east end, y in the
    middle, and r at the west — each in its own mouth of the ridge.
    Par 35, replay-verified route (three floors outrun the cap).
28. **Emberlight** — the dirt shaft is the hill: dig at once and punch
    from corridor to floor in one hole. Collect g and r below, ride
    your own tunnel up with a full line, then the ridge shuffle: two
    drops and two shoves re-stack the trail, and the west walk beds
    g, y and r in the order the beds lie, not the order the ants
    joined. Par 36, BFS optimum.
29. **Tinder Row** — all three ants in a row up top, all three beds
    mirrored on the middle row; a naive sweep locks nobody. Drop,
    shove, cross the island twice, and the final east pass delivers
    g, r and y onto their beds on three consecutive moves — the
    triple finish, landing warm. Par 39, BFS optimum.
30. **Ninefold** — a room whose only door is a bed. Take r from the
    top corridor, carry it the whole outer ring, enter through its
    own bed, deliver g and y inside on a two-loop shuffle, and walk
    back out over r: the step out of the room walks r onto R and
    locks the door behind you. Then the west shaft home. Par 41,
    replay-verified route (a hand-solved hill; neither BFS nor the
    beam ever found it).
31. **Deadfall** — the four-ant threshold, crossed gently: four beds,
    four dead-end slots, four ants waiting in the hall below. No wake
    lock exists anywhere on the hill; the whole answer is the push
    rhythm (shove, step in, shove home) four times over. Par 24,
    replay-verified route (BFS cannot settle four ants).
32. **Winter Quarters** — four ants swept up along the ridge, then the
    drops begin: leave two up east, descend and leave a third, walk the
    last one home by wake, ride the middle shaft up and let the trail
    re-form, then walk the middle corridor pushing the park row into
    its beds one shove at a time. Every warm bed reshapes the map under
    the next delivery. Par 48, replay-verified route.
33. **Deep Cellar** — three wake beds in a row plus one slot in the
    wall, and a colony that has to be un-loaded like freight: g parked
    mid-run, b walked up to its slot and posted with two pushes, then
    the ridge cleared with a triple drop and a closing shuttle that
    re-collects r off its own bed and lets it fall back asleep mid-
    stride. The finale before the finale. Par 51, replay-verified
    route.
34. **Ashfall** — the second four-ant run, every bed in the ridge.
    The colony is collected from two floors and walked up once; the
    two drops and three shoves in the long middle exist to get the
    trail *into order*, and the answer is the final ridge walk: b, g,
    y and r peel off onto their beds one after another, east to west,
    in your wake. Par 67, replay-verified route.
35. **Ember Deep** — the finale: four ants, a dirt gallery, and one
    continuous walk where the colony peels off b→g→y→r, each lock in
    your wake, none of them where the naive walk would drop them.
    Par 71, replay-verified route (four trail ants outrun BFS).

### Difficulty policy (solver-gated)

`solver.rs` runs BFS over the real state machine with exact state keys
(the full history deque is part of the identity — a truncated key was
tried and *empirically pruned true solutions*). A hill ships only when
a solution is **found and replayed** against a fresh `Game`. Where BFS
affords the full search, `par` is the proven optimum. Seven hills
outrun any cap we can carry — the five four-ant runs (Deadfall,
Winter Quarters, Deep Cellar, Ashfall, Ember Deep) plus Hearth Row
and Ninefold — so their pars are routes rather than proofs. Most come
from `hunt_route`, a layered beam search (width 30k, heuristic =
ant-to-bed distances plus a pull toward W, history-free dedup inside
the layer); Ninefold is a paper solution — the beam walks into the
bed-door too early and never finds it. A route is never claimed
optimal — only `verify` confers meaning, and every shipped route has
a `hand_route_wins_*` test replaying it move by move. `
solve_all_shipped` (--ignored) audits the whole lineup;
`probe_candidate_hills` (edit the candidate list, run with
`--ignored --nocapture`) is the design tool: paste a grid, get its
optimum and which verbs the solution uses; `hunt_candidate_hills`
does the same for hills only the beam can crack.

## 10. Scope for the 48h core milestone (success criteria)

- [x] Grid movement + tweened pixel motion in a loaded ASCII/RON level
- [x] Auto lock-in, bed-walls, LIFO peeling, leave-behind shuffle
- [x] Chain shift / jam rejection, Sokoban push + dunk shot
- [x] Dig with budget
- [x] Undo / restart / hill advance, HUD (digs left, warm count, moves/par)
- [x] Thirty-five hills, verified pars (28 BFS-proven, 7 replayed routes)
- [ ] Sprite swap when the atlas lands (rendering is isolated in `main.rs`)

## 11. Open questions (amend after playtest)

- Is "the line has nowhere to pack" legible, or does it read as an
  arbitrary veto? A ghost preview of the follower cells on hold might
  fix it; or soften to letting the rank wait behind.
- Is unconditional auto-collect too punishing (grab the wrong ant)?
  Escape hatch if needed: hold Ctrl to pass a WILD ant by.
- Par 71 for the finale — too long a walk? Test whether the multi-lap
  peeling reads as craft or as chore.
- Trail readability: number/rank pip on the tail ant so "who can leave
  the line" is obvious at a glance?
- Should riding ants (line compressed onto the player) lock in on a bed
  under the player's feet? Currently yes; verify it reads as intended.
