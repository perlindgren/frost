# Ember Hill — Walkthrough (optimal inputs, solver-proven)

Every sequence below is taken straight from the solver's replayed
output: for twenty-eight hills it is the **BFS optimum** (proven
minimal); for seven — the five four-ant runs (Deadfall, Winter
Quarters, Deep Cellar, Ashfall, Ember Deep) plus Hearth Row and
Ninefold — it is a **verified route**: found by the beam hunter
(Ninefold was solved on paper), replayed move by move every test run,
not claimed minimal. If any route ever stopped winning, the audit
would fail the build.

**Legend:** `← → ↑ ↓` arrow keys (WASD works too) · `⇧→` hold Shift + arrow
= **push** · `␣` Space = **leave the last ant behind** · digging is not a
key: walking an arrow into dirt digs it (one budget). Read `→×4` as
"press → four times".

---

## 1. First Warm — par 5

    →  ↑  ↑  ⇧→  ←

One push is the whole hill: step right, climb to the corridor beside r,
and push it **into** R — a push onto a matching bed dunks. Walk one step
home over your own (empty) bed. The colony's first verb, nothing else.

## 2. Attunement — par 5

    →×5

A straight hall: the empty R tile, then r, and W at the end. Walk right
and r falls asleep on R **in your footsteps** — the wake lock with no
steps to spare. The whole hill is the name: pass your ants home.

## 3. First Cold — par 9

    ←  ␣  ⇧→  →  ⇧→  ← ← ← ←

Grab r (←), immediately leave it behind (␣) so it sits on the corridor,
then post it into its bed with two pushes (⇧→ → ⇧→ — the second push
dunks it onto R). Walk home. This hill shows the dunk before it asks
you to need it.

## 4. Fire Escape — par 10

    →×3  ↑×4  ←×3

Start at the foot of the middle shaft with r one square above you.
Climb past r into the trail, ride the shaft to the top and step onto R;
your first step west leaves r tucked in the corner bed. Three steps to
W. First Cold, turned vertical.

## 5. Cheap Dig — par 10

    →×4  ↑  ↑  ←×4

Collect r, **dig straight up through the dirt cell** (first ↑), step onto
the R bed tile (second ↑), walk left: r packs onto its bed as you leave
and sleeps; you arrive at W. Around the long way costs fourteen — this
is the hill where paying the dig is the smart play.

## 6. Second Lap — par 16

    →×7  ↑×2  ←×7

Collect both ants walking right, loop up the east shaft, walk home.
On the way back the packed line peels **g onto G, then r onto R** — the
tail-swap double lock, for free, just by walking.

## 7. Split Paths — par 17

    ←×3  →×8  ↓×2  ←×4

You start between your two ants, and every bed sits between its ant and
home. Fetch g first (←×3), wheel east, collect r on the way past its
empty bed, and step onto G: r lands on R **while you stand on the other
ant's bed**, and g lands on G as you descend the shaft — wake locks
work on the way down too. Glide along the bottom to W.

## 8. Soft Soil — par 18

    →×8  ↑  ↑  ←×8

Collect b on the walk out. The east shaft is **dirt** — the first ↑ digs
through it (your only dig; it is the only way up). The second ↑ stands
on the empty B bed; the first ← home leaves b resting on it — locked.
Then walk the rest of the way to W.

## 9. Double Errand — par 20

    ↓×2  →×6  ↑×2  ←  →  ↑×2  ←×6

Down the west shaft and along the bottom: collecting y, your next few
steps west-to-east **wake-lock y onto Y** mid-walk (you pass its bed, it
rests in your wake). Up the east shaft, one step left onto r (collect),
one step right (a U-turn packs the line onto you), up to stand on the
empty R tile, then walk left: r drops onto R behind you and home.

## 10. Mail Slot — par 15

    →×4  ↑×2  ←  ␣  ⇧→  →  ⇧→  ←×4

R's bed is a **dead end** — corollary: no walking route can ever bed r
there. So: collect r, climb the middle shaft, step to the corridor
mouth, and Space drops r right behind you. Two pushes (⇧→ → ⇧→) post it
into the wall slot. Walk home.

## 11. Two Slots — par 29

    → →  ←  ↑ ↑  →  ␣  →  ↓ ↓  → →  ↑ ↑  ←  ␣  ⇧→  →  ⇧→  ← ← ←  ⇧←  ←  ↓×4  ←

Both beds are dead-end slots — Mail Slot, twice. Collect r, climb the
west shaft, and leave it at a slot mouth (␣). Loop the long way under
the map, collecting g, up the east shaft, leave g at its mouth too (␣):
two pushes post g into G (⇧→ → ⇧→), walk west along the ridge and one
push posts r into R (⇧←). Down the shaft and home. Never a wake lock —
here the only honest verb is the push.

## 12. Ridgeway — par 22

    →×10  ↑×2  ←×10

One out, one back, no repeated corridor: collect y then r along the
bottom, climb the far shaft onto R, and walk the ridge west — the tail
r peels onto R in your wake, y onto Y a few steps later. A Second Lap
with a view.

## 13. The Errand — par 22

    →×9  ↑×2  ←  ␣  ⇧→  ←×8

One ant, one dirt door, one far slot. Break through the dirt (the dig
is also a step), collect r, climb the far shaft, leave r behind at the
slot mouth (␣), and post it with a push (⇧→) — the bed is a dead end,
so walking never delivers. Then enjoy the long ridge home. The colony's
chore run: every verb, one visit each.

## 14. Doorway — par 22

    ↑×2  →×8  ↓×2  ←×8  ↑×2

R here is not an alcove — it is a **corridor cell**, currently an open
door. Climb to the ridge, stroll across your own (unoccupied) bed, ride
east, collect r, down the east shaft, and ride the bottom row west:
crossing R's cell in your wake locks r into it — and shuts the way you
came. Your way home is the shaft you started on. You closed the door
yourself; beds are load-bearing.

## 15. Two Ways Home — par 34

    →×5  ↓×2  ←×7  →×7  ↑×4  ←×9

Collect r at the corridor's end, drop down the east shaft, collect g on
the bottom run, and come home **up and over the top row**. Because g was
collected last, it peels first — onto G — then r onto R, then you onto
W. Collect them the other way around and R locks in front of G and the
hill is a tomb; this is the LIFO hill.

## 16. Pantry Row — par 34

    ↑×2  →×8  ↓×2  ←×5  →×5  ↑×4  ←×8

Cellar Row's calm sibling: down the west shaft, west along the bottom
picking up r, then y, loop east, climb the east shaft to the ridge and
walk home west. The tail **y lands on Y first**, then **r on R** —
collection order and bed order are exactly reversed, so it all just
walks.

## 17. Cellar Row — par 36

    →×2  ←×2  ↑×2  →×8  ↓  ␣  ⇧↑  ↓  ←×3  →×3  ↑×4  ←×8

Three ants, three beds, one long walk — with a shuttle hiding in it.
Collect g, haul it round the middle corridor to the east shaft foot,
drop it there (␣) and **push it one square up the shaft** (⇧↑) —
parked, not sleeping. Down to fetch r and y, back east, and climb the
shaft past the parked g to collect it again: the line re-forms as r, y,
g. Step onto the ridge and walk west: the tail peels **g onto G, then y
onto Y, then r onto R**, and W is your last step. The shuttle is not
decoration — it is how the optimum buys its tempo.

## 18. Three Cold Nights — par 41

    →×10  ␣  ↓  ␣ ␣  ⇧↑  ↑  ←  →  ⇧↑  ↓  ␣  ⇧↑  ↑  ←×3  →×3  ↑×2  ←×10

Collect all three (g, y, r) walking east. The **east-pocket shuttle**
then does the clever part: leave r behind in the corridor (␣, it waits
right where the line dropped it), drop down the shaft, leave y and g
too, and *push* them one at a time up into the top of the shaft column
(⇧↑ / ⇧↑) — parked, not sleeping. Walk back left (grabbing r again),
run right, and step up the shaft to re-collect y and g in that order —
the line re-forms as r, y, g. The final ←×10 along the top row is the
payoff: the tail peels **g onto G, then y onto Y, then r onto R**, and
you step onto W behind them. Nobody invents this route; you find it with
Z-undo, or you play it from here. It is the proven optimum.

## 19. Full Beds — par 31

    →×3  ⇧→  →  ␣  →  ↓×2  ←  ␣  ⇧→  →  ↑×2  ←×2  →×3  ←  ↓×2  ←×5  ↑×2  ←

Twenty open cells, and the middle corridor's beds are in the wrong
order for any one walk through them. Collect r, then shove y one cell
east (⇧→ — you will need it out of the way, and it ends up exactly
one shove from home). Drop r on the hall floor (␣) like freight you
will fetch later, take the east shaft down, and grab y where you left
it. Drop y again (␣) on the corner by G — and now the hill's signature:
**push east (⇧→) while you have been walking west**; y slides into Y
and the first bed closes. Climb up, walk back over your dropped r (it
rejoins the line), pick g up at the hall's end, and stride west along
the middle corridor — the wake peels **g onto G, then r onto R**, one
step apart. Up the west shaft, one step left, W. The solver proves no
30-move answer exists.

## 20. Ring Toss — par 33

    →  ←  ↓×2  →×6  ␣  →×2  ↑×2  ←×8  ↓×2  →×8

A ring with every bed on the return arc, and a lesson about lone
trailers: you collect r on your very first step, then walk the entire
bottom row east, past every bed — and r, following one cell behind,
never rests on anything long enough to sleep. ␣ on the R tile parks r
**right beside its own bed**, and you finish the circle the other way:
east shaft up, the whole top row west (collecting g, then y), west
shaft down. The final sweep east is the payoff: **y peels onto Y, g
onto G**, you trample your parked r back into the line, and standing
on R at last lets it fall asleep in your wake — one step onto W.
Two half-laps where one full lap looked like the answer.

## 21. Thimble — par 37

    ↓×2  →×5  ←×2  →×3  ↓×2  ←×3  →×3  ↑×2  ↓  ␣  ⇧↑  ↑  ⇧↑  ↓×2  ←×6  ↑×2

Nine columns, every verb. Down the west shaft, across the middle
corridor (past empty R and Y) to collect r at the east end — then step
**two cells back west**: the wake rewinds as you retrace, and r drifts
down onto R. The rewind is the rule nobody sees coming. Down the east
shaft to the floor, gather g then y walking west, trail them east and
up the shaft — **y peels onto Y mid-climb**. Only g is left, its bed a
dead-end hole in the top wall: drop it at the foot of the shaft (␣)
and post it up — ⇧↑, step in, ⇧↑ — dunked into the slot from below.
West along the floor, up the west shaft: W waits at the top.

## 22. Kindling — par 33

    →×5  ←×5  ↓↓  →×3  ␣  →×3  ↓↓  ←×6  ↑↑  ↓↓  →  ←

Three stacked corridors, two shafts at the ends. Take y and g on the
top sweep, return west, dive to the middle floor and take r in stride —
then leave it behind (␣) at once: the shortened tail packs tighter when
the floors fill. East shaft down, floor walk west: g, parked mid-floor,
settles onto its wall bed, and y peels off onto Y as you pass it. The
loop back up collects r again; the last steps walk it down onto R and
seat you on W beside it.

## 23. Cupboard — par 33

    →×6  ↑↑  ␣  ←×6  ↓×4  →×6  ↑↑  ↓  ↑×3  ←←

Ridge of bedrooms over a pocket corridor. Take r and g mid-hall, climb
the east shaft, and let go: g parks at the stair mouth, waiting to be
collected again. On the ridge walk west, drop a second time — this time
the tail lands *on its bed* and sleeps; dropping into bed locks just
like pushing into it. Down the west shaft, floor run east picking up y,
and the long ride back up: g slides off onto its floor bed, then y into
Y, one step before you seat yourself on W.

## 24. Soot Gate — par 33

    →×4  ←  ␣  →×3  ←←  ␣  ↓↓  ←←  ↑  ⇧↑  ↑  ←←  ↓  ␣  ⇧↑  ↑  ⇧↑  →→  ⇧↑  →→  ↑↑

Beds in the ceiling, g's bed alone down in the dirt row. The wall bed
takes g by plain walking; the two ceiling slots never sleep for feet —
corollary 1 — so they get the shopkeeper rhythm twice: park the ant at
the shaft foot, shove it up (⇧↑), step in, shove home. r first into the
west slot, y next into the middle one.

## 25. Slow Burn — par 32

    →×6  ↓↓  ←×6  ↓↓  →×5  ←×5  →×6

One serpentine, walking only — and the deepest lesson of the set. The
top sweep takes y; the west walk along the bedroom row beds y on Y in
your wake. Down to the floor, east to collect r and g, then walk back —
and the U-turn *reels the line up through old history*: g peels off
onto G, a bed two floors behind where you collected it. Walk the floor
east once more: r rides one step behind you the whole way and settles
onto R exactly one step before you reach W.

## 26. Pebble Gallery — par 34

    →×5  ↓×3  ␣  ↓  ←←  →×4  ␣  →  ↑×4  ←×3  ↓×4  ←×5

Two rings sharing one middle corridor, every bed on the other ring from
its ant. Take r up top, drop it partway down the east shaft, and work
the bottom ring: y and g fall in behind you. Another drop re-stacks the
tail; up the shafts and west along the ridge — y crosses the map into
its top-row bed — then down the middle and back for r at the corridor
mouth. The last west sweep beds g on G and r on R in two consecutive
moves, and the walk home is short.

## 27. Hearth Row — par 35 (verified route; optimality unknown by design)

    ↓↓  →  ←  ↑×4  →×4  ␣  ↓↓  ␣  ⇧↑  ↓↓  →×3  ↑×4  ←  ␣  ⇧→  ←×6

Every bed in the ceiling, ants living on the two lower floors, and a
trail that forms in the order the ridge will not accept. Gather — y
from the floor, r mid-hall, g from the floor's east end — and treat the
ridge as a workbench: two drops and a shove (⇧↑) re-stack the line,
then the west ridge walk is pure harvest: g at the east end, y in the
middle, r at the west, each sleeping in its own mouth of the ridge.

## 28. Emberlight — par 36

    →×5  ↓↓  ←←  →→  ↑×4  ←  ␣␣  ⇧→  →  ↓↓  →×3  ←×3  ↑↑  ⇧→  ←×5

The dirt shaft *is* the hill: dig straight down from the corridor and
you have a freight elevator — the floor pickups are one hole away.
Climb back through your own tunnel with the full line, then the ridge
shuffle: two drops and two shoves re-stack the trail on the ridge, and
the west walk beds g, y and r onto G, Y and R in bed order, not pickup
order. Your bed waits at the far end of your own sleeping colony.

## 29. Tinder Row — par 39

    ↓↓  →×8  ↑↑  ↓↓  ␣  ⇧←  ↑↑  ←×8  ↓↓  →×8  ↓↓  ←  ↓  ←

Ants lined up along the top, beds mirrored below them — the pickup
order is exactly backwards, so a naive sweep locks nothing. Dive, cross
to the far shaft, and take g off the ridge's east end; dropping and one
shove (␣ ⇧←) nudge it toward its bed. Then ride the whole ridge west,
collecting y and r, and the whole floor back east: the deep wake beds
g, and the trail follows with r onto R and y onto Y on the *two
following moves* — a triple finish, one move each. Then duck to your
bed.

## 30. Ninefold — par 41 (verified route; optimality unknown by design)

    →×6  ↑×5  ↓×5  ←×3  ↑×3  →  ↓↓  ←  ↑  ←  ↑  →  ↓×3  ←×3  ↑×5

The room with no door — except r's bed. Take r off the top corridor,
walk the whole outer ring down to the bottom row, and enter the room
*through* r's own bed tile: y and g are waiting inside. One small loop
in the room delivers g in the deep wake and y on the bed by the door,
and the step back out over the threshold walks r onto R — the door
falls asleep locked behind the colony. Bottom row west, shaft up, onto
W. The only hill in the set that neither the solver nor the beam ever
found on its own: this one was solved on paper, and the replay test is
its proof.

## 31. Deadfall — par 24 (verified route; optimality unknown by design)

    ⇧↑ ↑ ⇧↑   → ↓ →   ⇧↑ ↑ ⇧↑   → ↓ →   ⇧↑ ↑ ⇧↑   → ↓ →   ⇧↑ ↑ ⇧↑   → → ↓

Four beds, four dead-end slots, four ants sitting in the hall directly
underneath. This is the hill where you already know every verb — there
is just four of everything, and a wake lock never happens: no line
ever reaches a bed, because every ant is **pushed home**. The rhythm is
always the same three keys: shove the ant into the shaft (⇧↑), step
into the hole it left (↑), shove it into the slot (⇧↑) — then duck
back down (↓) and sidestep to the next ant (→ →). Same beat four times,
two steps home. The four-ant colony is terrifying until you notice
none of it is ever attached to you.

## 32. Winter Quarters — par 48 (verified route; optimality unknown by design)

    →×10  ␣␣  ↓×4  ␣  ←×4  ↑×4  → ← ↓↓ →  ␣  ⇧← ← ⇧← ← ⇧← ← ⇧←  →×6  ␣  ⇧←  ↑ ↑

The colony as freight, unloaded in three drops:

1. `→×10` — the ridge sweep: all four ants (b, g, y, r) fall in behind
   you, ending standing on the W tile. Nobody is ready yet.
2. `␣␣ ↓×4 ␣` — leave the tail behind, twice, up on the ridge; climb
   down the east shaft; leave one more at the bottom.
3. `←×4` — walk west along the south row: the last ant in line rests
   on **B** and the colony's first sleep happens mid-stride.
4. `↑×4 → ← ↓↓ → ␣` — ride the middle shaft up to the ridge where two
   parked ants rejoin the line, do a one-step shuffle, and leave the
   tail on the middle corridor.
5. `(⇧← ←)×3 ⇧←` — the shove parade: push, step into the gap, push
   again, four times west along the corridor, each shove delivering
   one parked ant toward its bed — r lands on R.
6. `→×6 ␣ ⇧←` — run back east, leave the last ant behind with one
   space-key, and shove it home into G. Colony asleep.
7. `↑↑` — one shaft up, onto W. Warm.

Every bed you close is a wall for what comes next: the route keeps
re-reading the map with one fewer open door.

## 33. Deep Cellar — par 51 (verified route; optimality unknown by design)

    →×5  ↑↑ →→ ␣ →→ ↓ ␣  ⇧↑ ↑ ⇧↑  ↓↓ ←×4  ↑×4  ←×3  ␣ ← ␣␣
    ← ↓↓ →×5 ↑↑  ← ␣ ⇧→  ←×4

The triple-deck unload — floor, middle, ridge, plus a slot in the
ridge wall:

1. `→×5` — the floor row: collect b and g.
2. `↑↑ →→ ␣ →→ ↓` — middle shaft up, east along the middle corridor,
   **leave g** where it is, keep going, drop down the far shaft.
3. `␣ ⇧↑ ↑ ⇧↑` — leave b in the shaft column, climb behind it and
   shove: two pushes post b up into the dead-end slot **B**. Dead ends
   get filled from below.
4. `↓↓ ←×4 ↑×4` — back down to the floor, collect r then y walking
   west, ride the middle shaft to the ridge — g rejoins the line on
   the climb.
5. `←×3 ␣ ← ␣␣` — walk the ridge west, **past your own W tile**, past
   every bed, to the west end — then leave all three trailing ants
   behind, parked in a row on the bed tiles. Freight first, delivery
   second.
6. `← ↓↓ →×5 ↑↑` — duck off the west shaft, cross the middle corridor
   east, come back up the middle: the ants you parked are now *behind*
   you.
7. `← ␣ ⇧→ ← ← ←` — the magic trick: one parked ant, one shove — g
   lands on **G**; a step west seats **Y**; walking onto R's tile
   re-collects r that was parked there, and the final westward steps
   let it fall back asleep **in your wake**, landing on R; the last
   step onto W ends the hill.

## 34. Ashfall — par 67 (verified route; optimality unknown by design)

    ↓↓  →×5  ←×5  ↑↑  →×3  ␣  →×5  ←×3  ↑↑  ←×3  ␣  ←  ␣␣  ←  ↓↓  ⇧→  →  ⇧→  →×4  ↑  ␣  ↑  ←  ␣  ⇧→  →  ⇧→  ↓↓  ↑↑  →  ⇧→  ←←  ␣  ⇧→  ←×4

The second four-ant run, every bed in the ridge. The colony is
collected in two armfuls — r and g from the floor, y and b from the
middle corridor's east end — and carried up once. Everything in the
long middle is logistics: drops and eastward shoves re-stack the trail
into bed order on the ridge. Then the harvest: the final west walk
peels b onto B at the east end, and g, y and r follow, one after
another, into the last eight moves of the hill — each in your wake,
each one step closer to W. Winter Quarters' lesson, done at length.

## 35. Ember Deep — par 71 (verified route; optimality unknown by design)

    →×12  ←×12  ↓×2  →×12  ↑×4  ←×12  ↓×2  →×12  ↓×2  ←

Five laps, each peeling one or two ants off the tail:

1. `→×12` — collect **all four** (r, y, g, b fall in behind you).
2. `←×12` — return lap; the packed line just follows you home.
3. `↓×2 →×12` — the bottom gallery: two digs in a row open the dirt
   ahead, you cross **B** — and b, at the tail, rests on it: first sleep.
4. `↑×4 ←×12` — up the east shaft and along the top row: **g peels onto
   G, then y onto Y**, both in your wake.
5. `↓×2 →×12 ↓×2 ←` — down, back along the middle row so **r lands on
   R** at your heels, then down to the W bed. Colony asleep, you warm.

---

*Sequences regenerate from the solver: `cargo test --release --example
antifrost solve_all_shipped -- --ignored --nocapture` prints the
twenty-eight BFS-optimal hills, and `hunt_candidate_hills -- --ignored
--nocapture` regenerates the beam-found routes
(`l r u d` walks, `L R U D` pushes, `x` Space); this file is that
output with the glyphs read as keystrokes.*
