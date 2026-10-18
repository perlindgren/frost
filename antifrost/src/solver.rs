//! Solver: breadth-first search over the *real* game state machine,
//! with parent links so the winning node reconstructs to an action path.
//!
//! Every expansion drives the actual [`Game`], so reaching the win flag
//! is close to proof — and the returned path is re-applied to a fresh
//! game before it leaves `solution()`, making the proof airtight. The
//! BFS depth of a win is the true optimum: the `par` the HUD shows.
//!
//! Why not IDA*, asked properly: it would fix BFS's memory wall, but
//! only an ADMISSIBLE heuristic may prune an optimality search, and
//! three rules see to it that no useful one exists. A collect RE-SLOTS
//! the ant it takes along the existing history deque — a free relocation
//! to a cell the player passed long ago, for one action — so a distance
//! to bed bounds nothing (`the_wake_bound_lies_under_a_long_wake`).
//! One walk shifts the WHOLE line, every rank one step toward its own
//! bed, so a score summing per-ant distances charges the same move
//! several times. And the tail-only lock CASCADES: one action can warm
//! as many ants as the line is deep, which retires even the counting
//! bound "one move per ant still owed". The scores here are therefore
//! rankings for the beam, never bounds — measured, not believed, by
//! `probe_score_admissibility_on_proven_optima`: Cellar Row spends 26 of
//! its own 36 optimal states rated ABOVE its true remaining cost, and
//! Thimble overshoots by 8. The hills beyond BFS are accordingly hunted,
//! not proved, and every hunted route is trusted only through `verify`.
//!
//! The third failure is the one worth changing, and `hills_v2.ron` ships
//! the change: under `Wake::Short` the line remembers only as far as it is
//! deep and a recruit joins at the player's heels, so **no action can carry
//! an ant further than one cell** — which is precisely the property a
//! distance needs in order to bound anything. All three rules still own the
//! long-wake hills, and the same test that damns the bound there is what
//! licenses `wake_bound` here: `ida_star` prunes on it, and where it
//! finishes its answer is the optimum. `no_short_wake_action_moves_an_ant_
//! further_than_one_cell` in `game` is the rule-level half of that claim.

use crate::game::{AntState, Dir, Game};
use crate::level::Level;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiply-shift hasher: keys are short byte strings and SipHash's cost
/// per state dominates the search.
#[derive(Default)]
struct Fold(u64);

impl Hasher for Fold {
    fn write(&mut self, bytes: &[u8]) {
        let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ bytes.len() as u64;
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
        }
        self.0 = h ^ (h >> 31);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FoldSet = HashMap<u64, (), BuildHasherDefault<Fold>>;

/// Multiply-shift hasher over the key itself, plus identity reuse of the
/// finished hash as a map key (collisions can only merge two states,
/// which never invents a win — every reported path is replay-checked).
#[inline]
fn fold_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ bytes.len() as u64;
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
    }
    h ^ (h >> 31)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Move(Dir),
    Push(Dir),
    Detach,
}

impl Act {
    /// Short glyph for solution dumps: lowercase move, uppercase push,
    /// `x` detach (bed the tail).
    pub fn glyph(self) -> char {
        use Dir::*;
        match self {
            Act::Move(Left) => 'l',
            Act::Move(Right) => 'r',
            Act::Move(Up) => 'u',
            Act::Move(Down) => 'd',
            Act::Push(Left) => 'L',
            Act::Push(Right) => 'R',
            Act::Push(Up) => 'U',
            Act::Push(Down) => 'D',
            Act::Detach => 'x',
        }
    }
}

const ALL: [Act; 9] = [
    Act::Move(Dir::Left),
    Act::Move(Dir::Right),
    Act::Move(Dir::Up),
    Act::Move(Dir::Down),
    Act::Push(Dir::Left),
    Act::Push(Dir::Right),
    Act::Push(Dir::Up),
    Act::Push(Dir::Down),
    Act::Detach,
];

fn apply(g: &mut Game, a: Act) -> bool {
    let ok = match a {
        Act::Move(d) => g.move_player(d).is_ok(),
        Act::Push(d) => g.push(d).is_ok(),
        Act::Detach => g.detach().is_ok(),
    };
    g.forget_history();
    ok
}

/// Grid cell → u8 (jam hills are far below 256 cells).
fn cell_i(g: &Game, c: crate::game::Cell) -> u8 {
    (c.row * g.level.cols as i32 + c.col) as u8
}

/// The state key keeps the FULL history: trails pack into deep history
/// cells during retrace chains, so truncating it — tried and rejected —
/// prunes real solutions and misreports par (it made First Cold look
/// like 16 instead of the true 15). Exact keys cost memory; the arena
/// and hash-only `seen` pay for them.
fn encode(g: &Game) -> Vec<u8> {
    let mut v = Vec::with_capacity(48);
    v.push(cell_i(g, g.player));
    v.push(g.won as u8);
    // Dug cells: one bit per dirt cell of the level.
    let mut mask: u8 = 0;
    let mut bits = 0;
    for i in 0..g.level.tiles.len() {
        if g.level.tiles[i] == crate::level::Tile::Dirt {
            if g.tiles[i] != crate::level::Tile::Dirt {
                mask |= 1 << bits;
            }
            bits += 1;
            if bits == 8 {
                v.push(mask);
                mask = 0;
                bits = 0;
            }
        }
    }
    if bits > 0 {
        v.push(mask);
    }
    // History: the full deque — see the exactness note above.
    v.push(g.history.len().min(u8::MAX as usize) as u8);
    for c in g.history.iter().take(u8::MAX as usize) {
        v.push(cell_i(g, *c));
    }
    v.push(g.trail.len() as u8);
    for &id in &g.trail {
        v.push(id as u8);
    }
    for ant in &g.ants {
        let (tag, cell) = match ant.state {
            AntState::Wild(c) => (1u8, Some(c)),
            AntState::Trail => (0, None),
            AntState::Placed(c) => (2, Some(c)),
            AntState::Asleep(c) => (3, Some(c)),
        };
        v.push(tag);
        if let Some(c) = cell {
            v.push(cell_i(g, c));
        }
    }
    v
}

fn decode(level: &Level, e: &[u8]) -> Game {
    let mut g = Game::start(level.clone());
    let mut i = 0usize;
    fn at(e: &[u8], i: &mut usize) -> u8 {
        let v = e[*i];
        *i += 1;
        v
    }
    let cols = level.cols as i32;
    let cell = |v: u8| crate::game::Cell::new(v as i32 % cols, v as i32 / cols);
    g.player = cell(at(e, &mut i));
    g.won = at(e, &mut i) != 0;
    let dirt: Vec<usize> = (0..level.tiles.len())
        .filter(|&t| level.tiles[t] == crate::level::Tile::Dirt)
        .collect();
    if !dirt.is_empty() {
        let mut k = 0;
        while k < dirt.len() {
            let mask = at(e, &mut i);
            for b in 0..8 {
                if k < dirt.len() && (mask >> b) & 1 == 1 {
                    g.tiles[dirt[k]] = crate::level::Tile::Tunnel;
                }
                k += 1;
            }
        }
    }
    let hlen = at(e, &mut i) as usize;
    g.history = (0..hlen).map(|_| cell(at(e, &mut i))).collect();
    let tlen = at(e, &mut i) as usize;
    g.trail = (0..tlen).map(|_| at(e, &mut i) as usize).collect();
    for ant in &mut g.ants {
        let tag = at(e, &mut i);
        ant.state = match tag {
            0 => AntState::Trail,
            1 => AntState::Wild(cell(at(e, &mut i))),
            2 => AntState::Placed(cell(at(e, &mut i))),
            _ => AntState::Asleep(cell(at(e, &mut i))),
        };
    }
    g.forget_history();
    g
}

/// One BFS run. Encoded states live in one flat arena (no per-node
/// allocations); `seen` keeps only their 64-bit hashes.
pub struct Run {
    arena: Vec<u8>,
    /// (byte offset, byte len, parent node, action taken from parent).
    meta: Vec<(u32, u16, u32, Act)>,
    pub winner: Option<u32>,
    /// True when the state cap ended the search rather than exhaustion.
    pub capped: bool,
}

impl Run {
    fn key(&self, i: usize) -> &[u8] {
        let (off, len, ..) = self.meta[i];
        &self.arena[off as usize..(off + len as u32) as usize]
    }
}

fn push_node(run: &mut Run, e: &[u8], parent: u32, act: Act) -> u32 {
    let off = run.arena.len() as u32;
    run.arena.extend_from_slice(e);
    run.meta.push((off, e.len() as u16, parent, act));
    run.meta.len() as u32 - 1
}

/// One BFS run, expanding no deeper than `max_depth` — `usize::MAX`
/// for "prove me everything". A finite bound turns BFS from solver
/// into CERTIFIER: BFS expands level by level, so a found win is at
/// minimum depth (reaching it exhausted every shallower layer), and
/// an exhaustion WITHOUT a win up to the bound proves no win that
/// short exists. Hand the bound one below a known route and these
/// two facts bracket a proof of optimality — which is exactly what
/// `solve` does with the hunter's route.
pub fn bfs(level: &Level, max_states: usize, max_depth: usize) -> Run {
    let start = Game::start(level.clone());
    let start_enc = encode(&start);
    let mut seen: FoldSet = FoldSet::default();
    seen.insert(fold_key(&start_enc), ());
    let mut run = Run {
        arena: Vec::new(),
        meta: Vec::new(),
        winner: None,
        capped: false,
    };
    push_node(&mut run, &start_enc, u32::MAX, Act::Detach);
    if start.won {
        run.winner = Some(0);
        return run;
    }
    let mut frontier = vec![0u32];
    let mut depth = 0usize;
    loop {
        depth += 1;
        let mut next = Vec::new();
        let mut capped = false;
        'expand: for &idx in &frontier {
            let base = decode(level, run.key(idx as usize));
            for a in ALL {
                let mut g = base.clone();
                if !apply(&mut g, a) {
                    continue;
                }
                let e = encode(&g);
                if seen.insert(fold_key(&e), ()).is_some() {
                    continue;
                }
                if run.arena.len() + e.len() > 3_800_000_000 {
                    // u32 offsets in meta; a hill with huge histories can
                    // hit this wall long before its state cap. Report it
                    // honestly as capped rather than wrap and corrupt.
                    capped = true;
                    break 'expand;
                }
                let me = push_node(&mut run, &e, idx, a);
                if g.won {
                    run.winner = Some(me);
                    break 'expand;
                }
                if run.meta.len() >= max_states {
                    capped = true;
                    break 'expand;
                }
                next.push(me);
            }
        }
        if run.winner.is_some() || capped || next.is_empty() {
            run.capped = capped;
            if capped {
                log::info!("solver: {max_states} states reached at depth {depth}");
            }
            return run;
        }
        if depth >= max_depth {
            // The bound is reached with children still queued: this is
            // an EXHAUSTION within the depth, not a cap. The caller
            // reads a clean stop as "no win this short", which is the
            // certificate `solve` asks for. A win exactly at the bound
            // was found by the loop above, so it is not missed.
            return run;
        }
        frontier = next;
    }
}

/// Walk the parent links of a winning node back to the root.
pub fn path(run: &Run) -> Vec<Act> {
    let mut w = run.winner.expect("run has no winner");
    let mut p = Vec::new();
    loop {
        let (parent, act) = (run.meta[w as usize].2, run.meta[w as usize].3);
        if parent == u32::MAX {
            break;
        }
        p.push(act);
        w = parent;
    }
    p.reverse();
    p
}

/// `lrdLURx…` → actions (inverse of `glyph`), for replaying solutions.
pub fn parse(s: &str) -> Vec<Act> {
    use Act::*;
    s.chars()
        .map(|c| {
            let d = |dir| {
                if c.is_uppercase() {
                    Push(dir)
                } else {
                    Move(dir)
                }
            };
            match c.to_ascii_lowercase() {
                'l' => d(Dir::Left),
                'r' => d(Dir::Right),
                'u' => d(Dir::Up),
                'd' => d(Dir::Down),
                'x' => Detach,
                other => panic!("bad glyph {other:?}"),
            }
        })
        .collect()
}

/// Re-apply a path to a fresh game; the game itself is the proof.
pub fn verify(level: &Level, p: &[Act]) -> bool {
    let mut g = Game::start(level.clone());
    for &a in p {
        if !apply(&mut g, a) {
            return false;
        }
    }
    g.won
}

/// The optimal solution (BFS depth = optimum), verified by replay, or
/// `None` if the cap stopped the search before any win.
pub fn solution(level: &Level, max_states: usize) -> Option<Vec<Act>> {
    let run = bfs(level, max_states, usize::MAX);
    run.winner.map(|_| path(&run)).filter(|p| verify(level, p))
}

/// What the unified solver leaves behind: the shortest route anyone
/// found — always replay-verified — and whether its length is PROVEN
/// unbeatable or merely the best a beam walked.
pub struct Solved {
    pub route: Vec<Act>,
    pub proven: bool,
}

/// One door, two minds.
///
/// The beam goes first: it finds a route fast where BFS cannot get a
/// foothold, and — the reason to unite them — its length is exactly
/// the bound a proof needs. Bounding BFS one below the hunted route
/// turns it from solver into CERTIFIER. A BFS win at or below the
/// bound is shorter and OPTIMAL (BFS expands level by level, so
/// reaching it exhausted every shallower layer); a BFS EXHAUSTION
/// with no win below the bound proves the hunted route unbeatable.
/// Both verdicts demand the real game replay the route, and neither
/// is claimed if BFS hit its state cap first — a cap is silence, not
/// proof. When the beam finds nothing at all, BFS runs unbounded as
/// the honest fallback: any win it reaches is proven optimal, and a
/// capped nothing means the hill has not spoken.
pub fn solve(level: &Level, max_states: usize) -> Option<Solved> {
    if let Some(route) = hunt_route(level, 150) {
        // Certify: no win shorter than the route, explored to its end.
        let run = bfs(level, max_states, route.len().saturating_sub(1));
        let shorter = run.winner.map(|_| path(&run)).filter(|p| verify(level, p));
        if let Some(shorter) = shorter {
            return Some(Solved {
                route: shorter,
                proven: true,
            });
        }
        // No shorter win, BFS did not cap, and the route really wins:
        // the hunt is optimal. Anything else — a cap, or a route the
        // game will not replay — is an unproven best, never a proof.
        let proven = run.winner.is_none() && !run.capped && verify(level, &route);
        return Some(Solved { route, proven });
    }
    // The beam never reached the summit: plain BFS, whose win is proven.
    let run = bfs(level, max_states, usize::MAX);
    run.winner
        .map(|_| path(&run))
        .filter(|p| verify(level, p))
        .map(|route| Solved {
            route,
            proven: true,
        })
}

/// `lrdLURx…` action string for eprintln.
pub fn glyphs(p: &[Act]) -> String {
    p.iter().map(|a| a.glyph()).collect()
}

/// Which mechanics an optimal path actually uses — tells a designer
/// whether a hill really teaches push / detach / digging.
pub fn used(p: &[Act]) -> String {
    let mut u = String::new();
    for (tag, want) in [
        ("walk", p.iter().any(|a| matches!(a, Act::Move(_)))),
        ("bed", p.contains(&Act::Detach)),
        ("push", p.iter().any(|a| matches!(a, Act::Push(_)))),
    ] {
        if want {
            u.push_str(tag);
            u.push(' ');
        }
    }
    u
}

// ---------------------------------------------------------------------------------------
// The searched physics: a bound you may prune on, and IDA* that prunes on it.
//
// Both halves hold only under `Wake::Short`, where every ant moves at most
// one cell per action. That one property is what the long wake throws away,
// and it is the whole difference between pruning a branch that cannot hide
// a win and deleting the win.
// ---------------------------------------------------------------------------------------

/// One BFS field per bed: the shortest walk to that bed through everything
/// the hill's FIXED rock allows — dirt counts as walkable, because dirt
/// yields to a dig and this relaxation prices that dig at nothing, and a
/// bed not warmed yet is just floor. Rock is the only wall that never
/// moves, so one field per bed serves every state of every run.
///
/// Read them as the map a delivery could never beat, never as the map it
/// must take: warm beds, standing ants and a jamming line all cost nothing
/// here. Which is why the honest thing to do with them is take a maximum.
pub fn distance_fields(level: &Level) -> Vec<Vec<i32>> {
    use std::collections::VecDeque;
    level
        .beds
        .iter()
        .map(|(from, _)| {
            let mut d = vec![-1i32; level.tiles.len()];
            let start = from.row as usize * level.cols + from.col as usize;
            d[start] = 0;
            let mut q = VecDeque::from([start]);
            while let Some(c) = q.pop_front() {
                let (r, k) = ((c / level.cols) as i32, (c % level.cols) as i32);
                for (dr, dk) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (nr, nk) = (r + dr, k + dk);
                    if nr < 0 || nk < 0 || nr as usize >= level.rows || nk as usize >= level.cols {
                        continue;
                    }
                    let n = nr as usize * level.cols + nk as usize;
                    if d[n] == -1 && level.tiles[n] != crate::level::Tile::Rock {
                        d[n] = d[c] + 1;
                        q.push_back(n);
                    }
                }
            }
            d
        })
        .collect()
}

/// The bound a short-wake search prunes on: **the farthest thing still owed
/// a place**, read off those fields.
///
/// Every unwarmed ant must come to rest on its own bed, and under a short
/// wake every ant moves at most one cell per action — a walk carries the
/// whole line but carries it one cell, a U-turn shifts it one cell, a
/// collect relocates its ant the single step from the cell it stood on to
/// the cell the player left, and the leave-behind moves its ant not at all.
/// Three families of term follow, and each is a lower bound on the moves
/// left:
///
/// - an ant's own distance to its bed, since it covers one cell per action;
/// - for a stranded (wild or left-behind) ant, that distance *plus* the
///   player's distance to it: it moves nothing until someone arrives, so
///   the walk to it and the walk it owes are two walks, not one;
/// - for any ant still owed, the player's walk home from that ant's bed
///   afterwards, because the sleep happens before the player sleeps and
///   never after it;
/// - the player's own distance to `W`, and to each owed bed, since an ant
///   comes to rest on a bed only in a cell the player has just stood in.
///
/// Two of those chains shave one and one shaves two: a dunk leaves the
/// player one cell short of the ant, and again one cell short of the bed,
/// and nothing in the rules forces them any closer. Nothing else is
/// rounded, capped or discounted — that restraint is the whole content of
/// admissibility, and since no term can fall faster than one per action the
/// maximum is *consistent* too: `f` never decreases along a path, which is
/// what stops IDA* from re-walking ground it has already priced.
///
/// Deliberately not a sum: one walk shifts the whole line at once, so
/// adding the ants' distances charges the same move several times over.
/// `sharp_score` is that sum, and it is a fine way to rank a beam and a
/// dishonest way to prune — `the_wake_bound_lies_under_a_long_wake`
/// is the hill that shows the difference, and it shows it under the long
/// wake, where even the maximum lies.
pub fn wake_bound(level: &Level) -> impl Fn(&Game) -> i32 + '_ {
    let fields = distance_fields(level);
    let idx = |c: crate::game::Cell| c.row as usize * level.cols + c.col as usize;
    let bed_at = move |c: crate::game::AntColor| {
        level
            .beds
            .iter()
            .position(|(_, k)| *k == c)
            .expect("every ant color has a bed (level validator)")
    };
    let white = bed_at(crate::game::AntColor::White);
    let sealed = |d: i32| if d < 0 { 9_999 } else { d };
    // How far each bed sits from the player's bed, and from every other bed:
    // once per hill, because the beds are the one part of it that never
    // moves. These two tables are the whole route term.
    let home: Vec<i32> = level
        .beds
        .iter()
        .map(|(cell, _)| sealed(fields[white][idx(*cell)]))
        .collect();
    let between: Vec<Vec<i32>> = (0..level.beds.len())
        .map(|i| {
            level
                .beds
                .iter()
                .enumerate()
                .map(|(j, (cell, _))| {
                    if i == j {
                        0
                    } else {
                        sealed(fields[i][idx(*cell)])
                    }
                })
                .collect()
        })
        .collect();
    move |g: &Game| -> i32 {
        // Distance from any cell to the bed of `color`, on the rock-only map.
        let dist = |color: crate::game::AntColor, from: crate::game::Cell| -> i32 {
            sealed(fields[bed_at(color)][idx(from)])
        };
        // The player owes their own bed, and owes it last.
        let mut h = dist(crate::game::AntColor::White, g.player);
        for (rank, &id) in g.trail.iter().enumerate() {
            let color = g.ants[id].color;
            let owed = dist(color, g.trail_target(rank));
            let bed = bed_at(color);
            h = h.max(owed);
            // Somebody has to walk onto that bed, and a line ant can only
            // bed that way: the trail's ants are nobody's to push. And when
            // it happens the player is standing on that bed or one cell off
            // it — only the tail peels, and the tail rests where the player
            // just was — so the walk home from there is owed on top.
            h = h.max((dist(color, g.player) - 1).max(0));
            h = h.max(owed + (home[bed] - 1).max(0));
        }
        for ant in &g.ants {
            let cell = match ant.state {
                AntState::Wild(c) | AntState::Placed(c) => c,
                _ => continue,
            };
            let owed = dist(ant.color, cell);
            let bed = bed_at(ant.color);
            // Somebody has to walk onto that bed: an ant rests on a bed
            // only in the cell the player just left.
            h = h.max((dist(ant.color, g.player) - 1).max(0));
            // Stranded: it moves nothing until someone arrives, so the
            // walk to it and the walk it owes are two walks, not one.
            // Straight-line for the reach, because the fields run to beds
            // and a straight line never overstates a hill.
            let steps = (g.player.col - cell.col).abs() + (g.player.row - cell.row).abs();
            let reach = (steps - 1).max(0);
            h = h.max(owed + reach);
            // …and the player still has to get home from that bed after it.
            h = h.max(owed + reach + (home[bed] - 1).max(0) - 1);
        }
        // The route term: a run's walk starts where the player stands, calls
        // at every bed still owed, and ends on `W`. Strip every blocker and
        // the whole ordering of the line out of that walk and what is left is
        // the shortest path through those places — computed here exactly, by
        // trying every order over at most four beds, since a walk is a path
        // and a path has an order.
        //
        // Each bed is a place the player must come *within two cells* of,
        // never a place they must stand on, because the dunk leaves them
        // exactly that far short; so each leg loses the slack of both its
        // ends. `W` is exempt — nobody dunks the player.
        let mut owed: [usize; 4] = [white; 4];
        let mut n = 0usize;
        for ant in &g.ants {
            if matches!(ant.state, AntState::Asleep(_)) {
                continue;
            }
            let bed = bed_at(ant.color);
            if !owed[..n].contains(&bed) && n < 4 {
                owed[n] = bed;
                n += 1;
            }
        }
        h.max(shortest_tour(&between, white, &owed[..n], |bed| {
            sealed(fields[bed][idx(g.player)])
        }))
        .max(0)
    }
}

/// The cheapest walk that starts at the player's cell, calls at every bed in
/// `owed`, and ends on `W` — the shape of a run's route with every blocker,
/// every ant and the whole ordering of the line stripped out of it. Computed
/// exactly, by walking every order: four beds is twenty-four orders, which is
/// cheaper than being clever about it, and it bounds a walk as well as a path
/// bounds a walk.
///
/// A bed the dunk could fill is a place the player must come *within two
/// cells* of, never a place they must stand on, so each leg loses the slack of
/// both its ends; `W` takes none, because nobody dunks the player.
fn shortest_tour(
    between: &[Vec<i32>],
    white: usize,
    owed: &[usize],
    from_player: impl Fn(usize) -> i32,
) -> i32 {
    let slack = |bed: usize| if bed == white { 0 } else { 2 };
    let start = |bed: usize| (from_player(bed) - slack(bed)).max(0);
    if owed.is_empty() {
        return 0;
    }
    let legs = Legs {
        between,
        white,
        owed,
    };
    let mut best = i32::MAX;
    for (i, &bed) in owed.iter().enumerate() {
        legs.walk(start(bed), bed, 1 << i, &mut best);
    }
    best
}

/// The distances a candidate order is priced with, and the beds it may
/// visit: a leg's cost is the relaxed walk between two beds, minus the slack
/// of both ends.
struct Legs<'a> {
    between: &'a [Vec<i32>],
    white: usize,
    owed: &'a [usize],
}

impl Legs<'_> {
    fn leg(&self, a: usize, b: usize) -> i32 {
        let slack = |bed: usize| if bed == self.white { 0 } else { 2 };
        (self.between[a][b] - slack(a) - slack(b)).max(0)
    }

    /// One order, walked: `spent` cells so far, standing on `here`, the beds
    /// flagged in `taken` already called at. Every bed taken is one leg from
    /// finishing; the last is always `W`.
    fn walk(&self, spent: i32, here: usize, taken: usize, best: &mut i32) {
        if spent >= *best {
            return; // a cheaper order already priced this one out
        }
        let all = (1 << self.owed.len()) - 1;
        if taken == all {
            *best = (*best).min(spent + self.leg(here, self.white));
            return;
        }
        for (i, &bed) in self.owed.iter().enumerate() {
            if taken >> i & 1 == 0 {
                self.walk(spent + self.leg(here, bed), bed, taken | 1 << i, best);
            }
        }
    }
}

/// What IDA* spent, and whether it finished. An empty `route` with
/// `capped` false is a proof that the hill has no win at all; with
/// `capped` set it is only the budget running out.
pub struct Hunted {
    pub route: Vec<Act>,
    /// States visited in total, across every threshold.
    pub nodes: usize,
    /// How many thresholds the search climbed before it stopped.
    pub thresholds: usize,
    /// The node budget ended the search. Silence, not proof.
    pub capped: bool,
}

/// Where one depth-bounded walk ended.
enum Cut {
    /// A win, with the route to it left in `route`.
    Found,
    /// Nothing under the bound; this is the smallest `f` that exceeded it.
    Over(usize),
    /// The subtree ran out with no win and nothing pruned behind it.
    Done,
    /// The node budget ended the search. Silence, not proof.
    Capped,
}

/// Iterative-deepening A* over the real game, pruned by `wake_bound`: the
/// memory of one path instead of a whole layer, which is the trade IDA*
/// exists to make. Admissibility of the bound is what makes the first win
/// at a threshold the cheapest win there is; consistency is what keeps each
/// threshold's re-walk cheap instead of quadratic.
///
/// Sound only on a `Wake::Short` hill, and it says so in debug builds. On
/// the long-wake hills the same bound lies, and the honest tools there are
/// the beam and the bounded certifier above. Compare `nodes` (every state
/// across all thresholds) against `bfs`'s `visited` to see which search
/// carried a hill cheaper — the answer is what a heuristic is for.
pub fn ida_star(level: &Level, max_nodes: usize) -> Hunted {
    debug_assert!(
        matches!(level.wake, crate::game::Wake::Short),
        "ida_star prunes on wake_bound, which is only admissible under Wake::Short"
    );
    let h = wake_bound(level);
    let start = Game::start(level.clone());
    let mut route: Vec<Act> = Vec::new();
    let mut nodes = 0usize;
    let mut bound = h(&start).max(0) as usize;
    let mut thresholds = 0usize;
    loop {
        thresholds += 1;
        match deepen(&start, &h, 0, bound, max_nodes, &mut nodes, &mut route) {
            Cut::Found => {
                return Hunted {
                    route,
                    nodes,
                    thresholds,
                    capped: false,
                };
            }
            // Out of budget: nothing was shown either way.
            Cut::Capped => {
                return Hunted {
                    route: Vec::new(),
                    nodes,
                    thresholds,
                    capped: true,
                };
            }
            // Every branch closed under the bound and nothing was pruned:
            // no route exists at any depth.
            Cut::Done => {
                return Hunted {
                    route: Vec::new(),
                    nodes,
                    thresholds,
                    capped: false,
                };
            }
            Cut::Over(next) => bound = next,
        }
    }
}

/// One threshold's depth-first walk: `cost` actions deep, turning back the
/// moment `f` passes `bound`, and carrying a game per level of depth rather
/// than a layer of them.
fn deepen(
    g: &Game,
    h: &dyn Fn(&Game) -> i32,
    cost: usize,
    bound: usize,
    max_nodes: usize,
    nodes: &mut usize,
    route: &mut Vec<Act>,
) -> Cut {
    let f = cost + h(g).max(0) as usize;
    if f > bound {
        return Cut::Over(f);
    }
    if g.won {
        return Cut::Found;
    }
    let mut min = usize::MAX;
    for &a in ALL.iter() {
        let mut next = g.clone();
        if !apply(&mut next, a) {
            continue;
        }
        *nodes += 1;
        if *nodes > max_nodes {
            return Cut::Capped;
        }
        route.push(a);
        match deepen(&next, h, cost + 1, bound, max_nodes, nodes, route) {
            Cut::Found => return Cut::Found,
            Cut::Capped => return Cut::Capped,
            Cut::Over(t) => min = min.min(t),
            Cut::Done => {}
        }
        route.pop();
    }
    if min == usize::MAX {
        Cut::Done
    } else {
        Cut::Over(min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::from_grid;

    /// The new tutorial ring: BFS knows the shortest walk that warms r
    /// in the wake and still gets home before the bed walls close.
    #[test]
    fn first_cold_ring_optimum() {
        let rows = [
            "###########",
            "#W..r@.R..#",
            "#.#######.#",
            "#.........#",
            "###########",
        ];
        let level = from_grid("first cold ring", &rows, 0).expect("valid grid");
        let p = solution(&level, 200_000).expect("solvable");
        eprintln!("first cold ring: {} moves  {}", p.len(), glyphs(&p));
        assert_eq!(p.len(), 9, "drop r, dunk it, home — the shipped par");
    }

    #[test]
    fn unsolvable_is_proved_by_exhaustion() {
        // The red ant is collectable, but its bed lies behind solid rock:
        // the hill can never win, and the search exhausts the space.
        let rows = ["######", "#W.@r#", "######", "#R####"];
        let level = from_grid("probe", &rows, 0).expect("valid grid");
        let run = bfs(&level, 1_000_000, usize::MAX);
        assert!(run.winner.is_none());
        assert!(!run.capped, "small hill should exhaust, not cap");
    }

    /// The rule that forbids a distance-based prune, on a hill small
    /// enough to state whole. Six moves in, red and yellow ride the line
    /// and green is wild three cells from her bed — and TWO moves remain
    /// to win, because the seventh (stepping onto green) re-slots the
    /// whole line along the history deque and all three ants fall asleep
    /// from that one action. So the sum of ant-to-bed distances (5), its
    /// strongest repair the maximum (3), and the counting bound "one move
    /// per ant still owed a bed" (3) all sit ABOVE the true cost: each
    /// would prune the optimal branch, and an A*/IDA* built on any of
    /// them would report a `par` the hill can beat. A future heuristic
    /// that prices an ant by how far it stands from its bed must fail
    /// here first; that is what this test is for.
    /// The searched wing, each hill with the route it is proved at and the
    /// nodes its proof costs. The par in `hills_v2.ron` and the length of
    /// the route are the same number, and both searches agree on it — see
    /// `searched_hills_replay_and_the_bound_stays_honest` (every session)
    /// and `solve_all_short_wake` (on demand).
    const SEARCHED: [(&str, &str, usize); 5] = [
        ("First Light", "ururrd", 10_000),
        ("The Alcove", "urrrrrllxRd", 100_000),
        ("Heel Marks", "lulurrrrdrurdd", 1_000_000),
        ("Two Warm Walls", "lllulurrrrrrrrdrulllllddd", 60_000_000),
        (
            "Ash Hall",
            "rrrrdllddddrxLrrrrrrrrUuullxuurddrddllll",
            900_000_000,
        ),
    ];

    /// Two wings, two physics, neither drifting into the other. Every hill
    /// of the descent rides the long wake; every hill of the searched wing
    /// rides the short one. A file that starts disagreeing with itself — a
    /// `wake:` key appearing in the descent, a hill silently defaulting —
    /// is a hill whose par was proved under some other rule, so this is a
    /// test and not a tidiness pass.
    #[test]
    fn the_two_wings_keep_their_physics() {
        let descent = crate::level::load_levels(crate::LEVELS);
        let searched = crate::level::load_levels(crate::HILLS_V2);
        assert_eq!(searched.len(), 5, "the searched wing is a ladder of five");
        assert!(
            descent.iter().all(|l| l.wake == crate::game::Wake::Long),
            "a descent hill declared a short wake: its par was proved under the long one"
        );
        assert!(
            searched.iter().all(|l| l.wake == crate::game::Wake::Short),
            "a searched hill is being run under the long wake it was not written for"
        );
        for level in &searched {
            assert!(level.par > 0, "{} carries no par", level.name);
        }
    }

    /// The half of the searched wing's proof that runs every session: each
    /// route replays to a win in exactly its par, and along the way the
    /// bound never once prices a state above the moves really left. That
    /// second half is admissibility, checked where it matters most — on the
    /// optimum itself, one state at a time, against the known answer.
    #[test]
    fn searched_hills_replay_and_the_bound_stays_honest() {
        for (name, glyphs_of_route, _) in SEARCHED {
            let level = searched_hill(name);
            let route = parse(glyphs_of_route);
            assert_eq!(
                route.len(),
                level.par as usize,
                "{name}: route and par differ"
            );
            assert!(verify(&level, &route), "{name}: the proved route must win");
            let bound = wake_bound(&level);
            let mut g = Game::start(level.clone());
            for (step, &a) in route.iter().enumerate() {
                let left = level.par - step as i32;
                assert!(
                    bound(&g) <= left,
                    "{name}: the bound says {} moves remain after {step}, and only {left} do",
                    bound(&g),
                );
                assert!(apply(&mut g, a), "{name}: move {step} must land");
            }
            assert!(g.won, "{name}: the replay ends warm");
        }
    }

    /// A hill the long wake cannot be searched on and the short wake can,
    /// the same authored grid both times: the same bound the searched wing
    /// prunes on is simply false here. Three ants are owed and two moves
    /// will do, because one collect re-slots an ant down a road walked long
    /// ago and one action then warms the whole line — so the farthest bed
    /// is further away than the hill is long, and a search that believed
    /// the distance would prune the win away.
    #[test]
    fn the_wake_bound_lies_under_a_long_wake() {
        let rows = ["#########", "#.GYRgW.#", "#@ry...##", "#########"];
        let level = from_grid("re-slot", &rows, 0).expect("valid grid");
        let route = parse("rrlurrrr");
        assert_eq!(route.len(), 8);
        assert!(verify(&level, &route), "the eight-move win must replay");

        // The state six moves in: three ants still owed, two moves left.
        let mut g = Game::start(level.clone());
        for &a in route.iter().take(6) {
            assert!(apply(&mut g, a), "the route to the probe state is legal");
        }
        let owed = g.ants.len() - g.ants_asleep();
        let remaining = (route.len() - 6) as i32;
        let (bound, score) = (wake_bound(&level)(&g), sharp_score(&level)(&g));
        assert_eq!(owed, 3, "three ants still owed a bed");
        assert_eq!(remaining, 2, "the collect, then one step onto W");
        for &a in route.iter().skip(6) {
            assert!(apply(&mut g, a), "the last two moves must land");
        }
        assert_eq!(g.ants_asleep(), 3, "ONE action warmed the whole line");
        assert!(g.won, "and two moves is all the hill needed");
        assert!(
            bound > remaining,
            "the very bound the short wake prunes on reads {bound} where {remaining} \
             moves remain: this is why the descent's hills are searched by BFS \
             and never pruned"
        );
        assert!(
            score > remaining,
            "and so does the beam's sum score, at {score}: it ranks, it does not prune"
        );
    }

    /// The walkthrough's source for the searched wing: each hill's proved
    /// route printed as keystrokes, with what the hill answers each move
    /// with — so the prose in `WALKTHROUGH.md` is a transcript and not a
    /// memory. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "print the searched wing's routes and their events"]
    fn walk_the_searched_wing() {
        for (name, glyphs, _) in SEARCHED {
            let level = searched_hill(name);
            let route = parse(glyphs);
            println!("\n{name} — par {} — `{glyphs}`", level.par);
            let mut g = Game::start(level.clone());
            for (step, &a) in route.iter().enumerate() {
                let (were_trail, were_asleep) = (g.trail.len(), g.ants_asleep());
                assert!(apply(&mut g, a), "{name}: move {step} must land");
                let where_: String = (0..g.ants.len())
                    .map(|id| {
                        let c = match g.ants[id].state {
                            AntState::Trail => "line".to_string(),
                            _ => format!("{:?}", standing_cell(&g, id)),
                        };
                        format!(" {:?} {c}", g.ants[id].color)
                    })
                    .collect();
                let warmed = g.ants_asleep() > were_asleep;
                let mut said = match g.trail.len().cmp(&were_trail) {
                    std::cmp::Ordering::Greater => "collected".to_string(),
                    std::cmp::Ordering::Less if warmed => "peeled".to_string(),
                    std::cmp::Ordering::Less => "left behind".to_string(),
                    std::cmp::Ordering::Equal => String::new(),
                };
                if warmed {
                    said.push_str(" WARM");
                }
                if g.won {
                    said.push_str(" WIN");
                }
                println!(
                    "  {:>2} {:<2} p({}, {}) trail {}{where_} {said}",
                    step + 1,
                    keys(a),
                    g.player.col,
                    g.player.row,
                    g.trail.len(),
                );
            }
            assert!(g.won, "{name}: the transcript must end warm");
        }
    }

    /// One move as the walkthrough writes it.
    fn keys(a: Act) -> String {
        match a {
            Act::Move(Dir::Left) => "←",
            Act::Move(Dir::Right) => "→",
            Act::Move(Dir::Up) => "↑",
            Act::Move(Dir::Down) => "↓",
            Act::Push(Dir::Left) => "⇧←",
            Act::Push(Dir::Right) => "⇧→",
            Act::Push(Dir::Up) => "⇧↑",
            Act::Push(Dir::Down) => "⇧↓",
            Act::Detach => "␣",
        }
        .to_string()
    }

    /// Where an ant stands in a state, however it is standing — the
    /// transcript reads the hill the way the player sees it.
    fn standing_cell(g: &Game, id: usize) -> crate::game::Cell {
        match g.ants[id].state {
            AntState::Wild(c) | AntState::Placed(c) | AntState::Asleep(c) => c,
            AntState::Trail => {
                let rank = g.trail.iter().position(|&t| t == id).expect("a rank");
                g.trail_target(rank)
            }
        }
    }

    /// The searched hill by name, from the shipped wing.
    fn searched_hill(name: &str) -> Level {
        crate::level::load_levels(crate::HILLS_V2)
            .into_iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("no searched hill named {name}"))
    }

    /// The expensive half, on demand:
    /// `cargo test -p antifrost --release -- solve_all_short_wake --ignored --nocapture --test-threads 1`.
    /// IDA* proves each par from the bound; exhaustive BFS, where its memory
    /// allows, settles the same number independently. Where BFS cannot
    /// settle the hill at all, IDA* still can — which is the whole argument
    /// for having the second search.
    #[test]
    #[ignore]
    fn solve_all_short_wake() {
        for (name, _proved, budget) in SEARCHED {
            let level = searched_hill(name);
            let h0 = wake_bound(&level)(&Game::start(level.clone()));
            let hunt = ida_star(&level, budget);
            let run = bfs(&level, 8_000_000, 220);
            let exact = (!run.capped).then(|| path(&run).len());
            assert!(
                !hunt.capped && hunt.route.len() as i32 == level.par,
                "{name}: IDA* was asked for a proof and came back after {} nodes \
                 with a route of {}",
                hunt.nodes,
                hunt.route.len()
            );
            assert!(
                verify(&level, &hunt.route),
                "{name}: the proved route must replay"
            );
            if let Some(d) = exact {
                assert_eq!(d as i32, level.par, "{name}: BFS and IDA* disagree");
            }
            println!(
                "{name:16} par {:3} | h0 {:3} | ida {:4} in {:>11} nodes, {:2} thresholds | bfs {} in {:>11} states",
                level.par,
                h0,
                hunt.route.len(),
                hunt.nodes,
                hunt.thresholds,
                exact.map(|d| d.to_string()).unwrap_or("—".into()),
                run.arena.len(),
            );
        }
    }

    /// The same failure where it actually bites: the SHIPPED score walked
    /// state by state along BFS-proven optimal paths. Each hill prints how
    /// many states of its own optimum the score rates above the TRUE
    /// remaining cost — every one of them a branch a prune on this score
    /// would delete on the way to the optimum — and how many edges drop
    /// the score by more than one move, which is the same untrustworthiness
    /// seen from the other side (a thin beam ranks a route's bad-looking
    /// middle out of the beam, and Ember Deep was lost that way at 30k).
    /// Run with `--ignored --nocapture`: it proves nothing, it keeps the
    /// module header's claim a measurement rather than a belief.
    #[test]
    #[ignore = "measure the shipped score against proven optima"]
    fn probe_score_admissibility_on_proven_optima() {
        for level in crate::level::load_levels(crate::LEVELS) {
            let name = level.name.clone();
            let run = bfs(&level, 24_000_000, usize::MAX);
            if run.winner.is_none() {
                eprintln!("{name}: BFS caps here, no optimum to measure against");
                continue;
            }
            let p = path(&run);
            let h = sharp_score(&level);
            let mut g = Game::start(level.clone());
            let (mut over, mut worst, mut drops) = (0usize, 0i32, 0usize);
            for (i, &a) in p.iter().enumerate() {
                let before = h(&g);
                assert!(apply(&mut g, a), "an optimal path is legal");
                let after = h(&g);
                let remaining = (p.len() - i - 1) as i32;
                if after > remaining {
                    over += 1;
                    worst = worst.max(after - remaining);
                }
                if before - after > 1 {
                    drops += 1;
                }
            }
            eprintln!(
                "{name}: optimum {} — score above the true cost on {over}/{} states \
                 (worst +{worst}), above-one drops on {drops}/{} edges",
                p.len(),
                p.len(),
                p.len()
            );
        }
    }

    #[test]
    #[ignore = "full solve of every shipped hill"]
    fn solve_all_shipped() {
        for level in crate::level::load_levels(crate::LEVELS) {
            let name = level.name.clone();
            let t = std::time::Instant::now();
            let run = bfs(&level, 24_000_000, usize::MAX);
            if run.winner.is_none() && !run.capped {
                panic!("{name}: UNSOLVABLE — a shipped hill must be winnable");
            }
            match run.winner {
                Some(_) => {
                    let p = path(&run);
                    assert!(verify(&level, &p), "{name}: replay failed");
                    assert_eq!(p.len() as i32, level.par, "{name}: par != BFS optimum");
                    eprintln!(
                        "{name}: par {} is optimal [{}]  {}",
                        p.len(),
                        used(&p),
                        glyphs(&p)
                    );
                }
                None => eprintln!(
                    "{name}: par {} — cap hit, no proof here; the hand_route_wins_* tests replay the route every run",
                    level.par
                ),
            }
            eprintln!("  {:?}", t.elapsed());
        }
    }

    /// BFS caps out on four trail ants (the history key is honest, so the
    /// space is enormous); this is the route the hunter retook at 100k
    /// width — it beat the five-lap 71 that had outlived every hunt
    /// before it. One big loop: gather on the middle row, a three-move
    /// juggle beds RED at the start, then each long ride peels one ant
    /// in its wake — BLUE along the bottom ride west, YELLOW then
    /// GREEN along the top ride east — and one step onto W after the
    /// east-shaft descent ends the hill.
    #[test]
    fn hand_route_wins_ember_deep() {
        let rows = [
            "###############",
            "#..Y.....G....#",
            "#.###########.#",
            "#@R.r.y..g..b.#",
            "#.###########.#",
            "#...B,,.....W.#",
            "###############",
        ];
        let level = from_grid("Ember Deep", &rows, 0).expect("valid grid");
        let glyphs = "rrrrrlllxrlrrrrrrrrrrddlllllllllllluuuurrrrrrrrrrrrddddl";
        let route = parse(glyphs);
        assert_eq!(route.len(), 56);
        assert!(verify(&level, &route), "the hand route must win");
    }

    #[test]
    #[ignore = "probe"]
    fn dbg_hunter_sanity() {
        let rows = [
            "#########",
            "#W.....R#",
            "#.###.###",
            "#@..r...#",
            "#.###.###",
            "#.......#",
            "#########",
        ];
        let level = from_grid("Mail Slot", &rows, 0).expect("valid grid");
        let route = super::hunt_route(&level, 200_000).expect("hunter should solve Mail Slot");
        eprintln!("hunter found {} moves: {}", route.len(), glyphs(&route));
        assert!(verify(&level, &route));
    }

    #[test]
    fn hand_route_wins_deadfall() {
        let rows = [
            "###########",
            "#R#Y#G#B###",
            "#.#.#.#.###",
            "#r.y.g.b..#",
            "#@.......W#",
            "###########",
        ];
        let level = from_grid("Deadfall", &rows, 0).expect("valid grid");
        let glyphs = "UuUrdrUuUrdrUuUrdrUuUrrd";
        let route = parse(glyphs);
        assert_eq!(route.len(), 24);
        assert!(verify(&level, &route), "the hand route must win");
    }

    /// The 100k hunt's 47 — one shorter than the freight route it
    /// replaced, and shaped differently: GREEN wakes on the first
    /// descent's ride, a drop-and-shove leaves an ant in the east
    /// shaft, a two-step juggle beds RED mid-hill, YELLOW wakes one
    /// step after being stood on, and BLUE comes home off a down-up
    /// beat at the east shaft — then straight up it to W.
    #[test]
    fn hand_route_wins_winter_quarters() {
        let rows = [
            "#############",
            "#@b..g..y.rW#",
            "#.#####.###.#",
            "#..R.....G..#",
            "#.#####.###.#",
            "#......Y..B.#",
            "#############",
        ];
        let level = from_grid("Winter Quarters", &rows, 0).expect("valid grid");
        let glyphs = "rrrrrrddrrrruxDullllddllllllxrrrrrrddrrrruduuuu";
        let route = parse(glyphs);
        assert_eq!(route.len(), 47);
        assert!(verify(&level, &route), "the hand route must win");
    }
    #[test]
    fn hand_route_wins_ninefold() {
        let rows = [
            "#########",
            "#W.....r#",
            "#.#####.#",
            "#.#YgG#.#",
            "#.#.y.#.#",
            "#.##R.#.#",
            "#@......#",
            "#########",
        ];
        let level = from_grid("Ninefold", &rows, 0).expect("valid grid");
        // The paper route was 41; the tunnel-aware hunt found 35.
        let glyphs = "rrrrrruuuuudddddlluuulldrddllluuuuu";
        let route = parse(glyphs);
        assert_eq!(route.len(), 35);
        assert!(verify(&level, &route), "ninefold route must win");
    }

    #[test]
    fn hand_route_wins_hearth_row() {
        let rows = [
            "###########",
            "#W.R..Y..G#",
            "#.###.##.##",
            "#@...r....#",
            "#.###.##.##",
            "#.y...g...#",
            "###########",
        ];
        let level = from_grid("Hearth Row", &rows, 0).expect("valid grid");
        let glyphs = "ddrluuuurrrrxddxUddrrruuuulxRllllll";
        let route = parse(glyphs);
        assert_eq!(route.len(), 35);
        assert!(verify(&level, &route), "hearth row route must win");
    }

    #[test]
    fn hand_route_wins_ashfall() {
        let rows = [
            "###########",
            "#W.R.Y.G.B#",
            "#.####.####",
            "#@.....y.b#",
            "#.####.####",
            "#..r..g...#",
            "###########",
        ];
        let level = from_grid("Ashfall", &rows, 0).expect("valid grid");
        // Ten moves off the old hunt's 67: the tunnel-aware score saw
        // the gallery the Manhattan beam walked past.
        let glyphs = "ddrrrrrxuurrrllluullllxxxlddddrrrrruuuuxlxRrRdurRllxRllll";
        let route = parse(glyphs);
        assert_eq!(route.len(), 57);
        assert!(verify(&level, &route), "ashfall route must win");
    }
    #[test]
    fn hand_route_wins_slow_burn() {
        let rows = [
            "#########",
            "#@....y.#",
            "#######.#",
            "#G....Y.#",
            "#.#######",
            "#..r.RgW#",
            "#########",
        ];
        let level = from_grid("Slow Burn", &rows, 0).expect("valid grid");
        let glyphs = "rrrrrrddllllllddrrrrrlllllrrrrrr";
        let route = parse(glyphs);
        assert_eq!(route.len(), 32);
        assert!(verify(&level, &route), "slow burn route must win");
    }

    #[test]
    fn hand_route_wins_deep_cellar() {
        // The hole the re-hunt closed: Deep Cellar shipped with par
        // 51 but no route test at all — the suite could not replay
        // its own number. This route replays it, asked of the
        // SHIPPED hill (not a grid copy) so par and route are tied:
        // if the par moves without a route to match, this test says
        // so. The player's own 48 still beats the route — the hunt
        // has not found it, only proven 51 replayable.
        let level = crate::level::load_levels(crate::LEVELS)
            .into_iter()
            .find(|l| l.name == "Deep Cellar")
            .expect("the shipped hill");
        let glyphs = "rrrrruurrxrrdxUuUddlllluuuulllxlxxlddrrrrruulxRllll";
        let route = parse(glyphs);
        assert_eq!(
            route.len(),
            level.par as usize,
            "par and route move together"
        );
        assert!(verify(&level, &route), "deep cellar route must win");
    }

    /// The unified door must be able to SAY proven, not only hunted:
    /// on hills small enough for BFS to settle below the hunted
    /// route, `solve` owes a proof AT par. Without this the proven
    /// branch ships unexercised — and a proof path no one walks is a
    /// proof path no one checked.
    #[test]
    fn the_unified_door_proves_the_hills_bfs_can_settle() {
        for name in ["First Warm", "First Cold", "Mail Slot", "Ridgeway"] {
            let level = crate::level::load_levels(crate::LEVELS)
                .into_iter()
                .find(|l| l.name == name)
                .expect("a shipped hill");
            let sol = super::solve(&level, 4_000_000).expect("these hills yield");
            assert!(
                sol.proven,
                "{name}: BFS settles this, so solve owes a proof"
            );
            assert_eq!(
                sol.route.len(),
                level.par as usize,
                "{name}: a proof at the wrong length is no proof"
            );
            assert!(verify(&level, &sol.route), "{name}: replay");
        }
    }

    #[test]
    fn key_round_trip_is_faithful() {
        let rows = ["#W..r@.R..#", "#.#######.#", "#.........#", "###########"];
        let level = from_grid("ring probe", &rows, 0).expect("valid grid");
        let mut g = Game::start(level.clone());
        let mut seed: u32 = 0x1234_5678;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) as usize
        };
        for step in 0..4000 {
            let enc = encode(&g);
            let back = decode(&level, &enc);
            let enc2 = encode(&back);
            assert_eq!(enc, enc2, "key not stable at step {step}");
            assert_eq!(g.player, back.player, "player at step {step}");
            assert_eq!(g.won, back.won, "won at step {step}");
            assert_eq!(g.tiles, back.tiles, "tiles at step {step}");
            assert_eq!(
                g.history.iter().copied().collect::<Vec<_>>(),
                back.history.iter().copied().collect::<Vec<_>>(),
                "history at step {step} (len {})",
                g.history.len()
            );
            assert_eq!(g.trail, back.trail, "trail at step {step}");
            for (a, b) in g.ants.iter().zip(&back.ants) {
                assert_eq!(a.color, b.color);
                assert_eq!(a.state, b.state, "ants at step {step}");
            }
            let _ = apply(&mut g, ALL[rnd() % ALL.len()]);
            if g.won {
                g = Game::start(level.clone());
            }
        }
    }

    /// Candidate hills under evaluation: par must be real before shipping.
    /// Paste grids here (name, rows, BFS state budget); the probe
    /// prints the optimal path, its length, and the mechanics it uses.
    /// Every candidate so far has graduated: Forced Hand and Deep Cellar
    /// shipped at their BFS optima, Fourposter and The Stranded shipped
    /// with hand-verified routes above, Long Winter died of state-space.
    #[test]
    #[ignore = "level design probe"]
    fn probe_candidate_hills() {
        let candidates: [(&str, &[&str], usize); 1] = [(
            "Slow Burn",
            &[
                "#########",
                "#@....y.#",
                "#######.#",
                "#G....Y.#",
                "#.#######",
                "#..r.RgW#",
                "#########",
            ],
            24_000_000,
        )];

        for (name, rows, cap) in candidates {
            let level = from_grid(name, rows, 0).expect("valid grid");
            let t = std::time::Instant::now();
            let run = bfs(&level, cap, usize::MAX);
            match run.winner {
                Some(_) => {
                    let p = path(&run);
                    assert!(verify(&level, &p), "{name}: replay failed");
                    eprintln!(
                        "{name}: {} moves [{}] {}  ({:?})",
                        p.len(),
                        used(&p),
                        glyphs(&p),
                        t.elapsed()
                    );
                }
                None => eprintln!(
                    "{name}: {}  ({:?})",
                    if run.capped {
                        "cap reached"
                    } else {
                        "UNSOLVABLE"
                    },
                    t.elapsed()
                ),
            }
        }
    }

    /// The four-ant set: BFS gasps on these, so the shipped par is
    /// whatever the route hunter finds and `verify` replays. Run with
    /// --ignored --nocapture to regenerate the shipped routes.
    #[test]
    #[ignore = "hunt routes for the four-ant hills"]
    fn hunt_candidate_hills() {
        let candidates: [(&str, &[&str]); 1] = [(
            "Kiln v2",
            &[
                "#########",
                "#R#Y#W..#",
                "#.#.#.###",
                "#..G.B.@#",
                "#,.#.#,.#",
                "#r.y.g.b#",
                "#########",
            ],
        )];
        for (name, rows) in candidates {
            let level = from_grid(name, rows, 0).expect("valid grid");
            let t = std::time::Instant::now();
            match super::hunt_route(&level, 150) {
                Some(route) => {
                    let ok = verify(&level, &route);
                    eprintln!(
                        "{name}: {} moves  verify={ok}  [{}]  {}  ({:?})",
                        route.len(),
                        used(&route),
                        glyphs(&route),
                        t.elapsed()
                    );
                    assert!(ok, "{name}: hunted route does not replay!");
                }
                None => eprintln!("{name}: NO ROUTE FOUND  ({:?})", t.elapsed()),
            }
        }
    }

    /// The unproven seven, put through the UNIFIED solver (`solve`).
    /// Run with `--ignored --nocapture`: each hill prints the route
    /// the door settled on, whether that route is now PROVEN optimal
    /// or merely the best hunted, its length beside the shipped par,
    /// and the replay verdict. Two events retune a hill: a route
    /// SHORTER than par (the par drops, the `hand_route_wins_*` test
    /// takes the new glyph) or a route turning PROVEN (the hill
    /// leaves the unproven seven).
    #[test]
    #[ignore = "solve the unproven seven, hunting and proving"]
    fn hunt_the_unproven_seven() {
        let seven = [
            "Hearth Row",
            "Ninefold",
            "Deadfall",
            "Winter Quarters",
            "Deep Cellar",
            "Ashfall",
            "Ember Deep",
        ];
        for level in crate::level::load_levels(crate::LEVELS) {
            if !seven.contains(&level.name.as_str()) {
                continue;
            }
            let t = std::time::Instant::now();
            match super::solve(&level, 24_000_000) {
                Some(sol) => {
                    let verdict = if sol.proven { "PROVEN" } else { "hunted" };
                    let flag = if sol.route.len() == level.par as usize {
                        String::new()
                    } else {
                        format!("  << par is {}!", level.par)
                    };
                    eprintln!(
                        "{}: {verdict} {} moves (par {})  [{}]  {}  ({:?}){flag}",
                        level.name,
                        sol.route.len(),
                        level.par,
                        used(&sol.route),
                        glyphs(&sol.route),
                        t.elapsed()
                    );
                    assert!(
                        verify(&level, &sol.route),
                        "{}: solved route does not replay!",
                        level.name
                    );
                }
                None => {
                    eprintln!(
                        "{}: NO ROUTE (par {})  ({:?})",
                        level.name,
                        level.par,
                        t.elapsed()
                    )
                }
            }
        }
    }
}

/// The beam's two minds. The sharp score floods the tunnels: for
/// each bed, BFS distance through everything the hill's FIXED rock
/// allows — dirt counts as walkable, because dirt yields to a dig
/// and the relaxation prices that dig at nothing. Rock is the only
/// wall that never moves, so one field per bed serves every state of
/// every run. The naive score is Manhattan, which walks through
/// walls — worse at estimating, yet Ember Deep proved it keeps roads
/// the sharp score prunes: a beam is not monotone in heuristic
/// quality, and the sharper estimate ranks a needed bad-looking
/// middle (the lapped ring, the dig past the bed) too confidently
/// out of the beam. So the hunter asks both, sharp first.
fn sharp_score(level: &Level) -> impl Fn(&Game) -> i32 + '_ {
    use std::collections::VecDeque;
    let fields: Vec<Vec<i32>> = level
        .beds
        .iter()
        .map(|(from, _)| {
            let mut d = vec![-1i32; level.tiles.len()];
            let start = from.row as usize * level.cols + from.col as usize;
            d[start] = 0;
            let mut q = VecDeque::from([start]);
            while let Some(c) = q.pop_front() {
                let (r, k) = ((c / level.cols) as i32, (c % level.cols) as i32);
                for (dr, dk) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (nr, nk) = (r + dr, k + dk);
                    if nr < 0 || nk < 0 || nr as usize >= level.rows || nk as usize >= level.cols {
                        continue;
                    }
                    let n = nr as usize * level.cols + nk as usize;
                    if d[n] == -1 && level.tiles[n] != crate::level::Tile::Rock {
                        d[n] = d[c] + 1;
                        q.push_back(n);
                    }
                }
            }
            d
        })
        .collect();
    let bed_idx = |c: crate::game::AntColor| {
        level
            .beds
            .iter()
            .position(|(_, k)| *k == c)
            .expect("every ant has a bed")
    };
    move |g: &Game| -> i32 {
        let dist = |color: crate::game::AntColor, at: crate::game::Cell| -> i32 {
            let d = fields[bed_idx(color)][at.row as usize * level.cols + at.col as usize];
            // Rock-sealed is a broken hill, not a hard one: a huge
            // score just sends the hunter looking elsewhere.
            if d < 0 { 9_999 } else { d }
        };
        let mut s = 0;
        for (i, &id) in g.trail.iter().enumerate() {
            s += dist(g.ants[id].color, g.trail_target(i));
        }
        for a in &g.ants {
            if let AntState::Wild(c) | AntState::Placed(c) = a.state {
                s += dist(a.color, c);
            }
        }
        if g.ants
            .iter()
            .all(|a| matches!(a.state, AntState::Asleep(_)))
        {
            s += 2 * dist(crate::game::AntColor::White, g.player);
        }
        s
    }
}

fn naive_score(level: &Level) -> impl Fn(&Game) -> i32 + '_ {
    let bed = |c: crate::game::AntColor| -> crate::game::Cell {
        *level
            .beds
            .iter()
            .find(|(_, k)| *k == c)
            .map(|(cell, _)| cell)
            .expect("every ant has a bed")
    };
    let man =
        |a: crate::game::Cell, b: crate::game::Cell| (a.col - b.col).abs() + (a.row - b.row).abs();
    move |g: &Game| -> i32 {
        let mut s = 0;
        for (i, &id) in g.trail.iter().enumerate() {
            s += man(g.trail_target(i), bed(g.ants[id].color));
        }
        for a in &g.ants {
            if let AntState::Wild(c) | AntState::Placed(c) = a.state {
                s += man(c, bed(a.color));
            }
        }
        if g.ants
            .iter()
            .all(|a| matches!(a.state, AntState::Asleep(_)))
        {
            s += 2 * man(g.player, bed(crate::game::AntColor::White));
        }
        s
    }
}

/// Route hunter for hills BFS cannot settle (four-ant trails). Layered
/// beam search over the REAL state machine: expand a frontier one ply,
/// score children by depth + 8 x heuristic, keep the best `width` of
/// them — sharp score first, naive score if that beam loses the road
/// (the two minds, above). Two honest compromises:
///
/// - the within-layer dedup key DROPS the history deque. That is not
///   sound for optimality (history gates packing legality), but this
///   tool never claims optimality: every route it returns must pass
///   `verify` before it means anything. A dropped merge can only make
///   the hunter miss routes, never invent one.
/// - a finite width can miss solutions that route through bad-looking
///   middles. Widen the beam or try again; the answer is trusted only
///   via replay anyway.
///
/// How many candidates the beam carries per layer. Cost is linear in
/// this — ~2.4 s per layer per 30k measured — and it buys recall:
/// 30k retook Ninefold and Ashfall but LOST Ember Deep's road (the
/// sharper score ranked the route's bad-looking middle out of a thin
/// beam, and the naive score at 30k did not re-find it either).
/// 100k triples the sweep cost (~20 min per hill per score when a
/// hill truly fails) to give that road a wider lane.
pub const BEAM_WIDTH: usize = 100_000;

pub fn hunt_route(level: &Level, max_depth: usize) -> Option<Vec<Act>> {
    let sharp = sharp_score(level);
    if let Some(route) = beam(level, max_depth, BEAM_WIDTH, &sharp) {
        return Some(route);
    }
    let naive = naive_score(level);
    beam(level, max_depth, BEAM_WIDTH, &naive)
}

fn beam(
    level: &Level,
    max_depth: usize,
    width: usize,
    h: &dyn Fn(&Game) -> i32,
) -> Option<Vec<Act>> {
    use std::collections::HashSet;

    // History-free state sketch: player, trail order, every ant's state.
    let sketch = |g: &Game| -> Vec<u8> {
        let mut v = Vec::with_capacity(24);
        v.push(cell_i(g, g.player));
        for &id in &g.trail {
            v.push(id as u8);
        }
        v.push(255);
        for a in &g.ants {
            let (tag, cell) = match a.state {
                AntState::Trail => (0u8, None),
                AntState::Wild(c) => (1, Some(c)),
                AntState::Placed(c) => (2, Some(c)),
                AntState::Asleep(c) => (3, Some(c)),
            };
            v.push(tag);
            if let Some(c) = cell {
                v.push(cell_i(g, c));
            }
        }
        v
    };

    struct Node {
        /// Frontier nodes own a game; once a node's layer has expanded it
        /// we DROP the game (route reconstruction only needs parent/act).
        /// Without this, the arena keeps a full Game per node — with undo
        /// chains that is O(depth^2 x width) memory and OOMs on 4-ant
        /// maps (learned via SIGKILL).
        game: Option<Game>,
        parent: usize,
        act: Option<Act>,
        f: i64,
    }
    let start = Game::start(level.clone());
    let mut arena: Vec<Node> = vec![Node {
        f: -(8 * h(&start) as i64),
        game: Some(start),
        parent: usize::MAX,
        act: None,
    }];
    let mut frontier: Vec<usize> = vec![0];
    const DIRS: [Dir; 4] = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];
    for depth in 0..max_depth {
        let mut next: Vec<usize> = Vec::with_capacity(frontier.len() * 4);
        let mut seen: HashSet<Vec<u8>> = HashSet::with_capacity(frontier.len());
        for &idx in &frontier {
            let Some(base) = arena[idx].game.take() else {
                continue;
            };
            let mut acts = Vec::with_capacity(9);
            for d in DIRS {
                acts.push(Act::Move(d));
                acts.push(Act::Push(d));
            }
            acts.push(Act::Detach);
            for act in acts {
                let mut ng = base.clone();
                let ok = match act {
                    Act::Move(d) => ng.move_player(d).is_ok(),
                    Act::Push(d) => ng.push(d).is_ok(),
                    Act::Detach => ng.detach().is_ok(),
                };
                if !ok {
                    continue;
                }
                if ng.won {
                    arena.push(Node {
                        f: 0,
                        game: Some(ng),
                        parent: idx,
                        act: Some(act),
                    });
                    let win = arena.len() - 1;
                    let mut route = Vec::new();
                    let mut cur = win;
                    while let Some(a) = arena[cur].act {
                        route.push(a);
                        cur = arena[cur].parent;
                    }
                    route.reverse();
                    return Some(route);
                }
                if !seen.insert(sketch(&ng)) {
                    continue;
                }
                arena.push(Node {
                    f: depth as i64 + 1 + 8 * h(&ng) as i64,
                    game: Some(ng),
                    parent: idx,
                    act: Some(act),
                });
                next.push(arena.len() - 1);
            }
        }
        next.sort_by_key(|&n| (arena[n].f, n));
        // The cut nodes are never re-expanded, so nothing would ever
        // drop their Game otherwise — the same O(nodes x Game) memory
        // wall the frontier-drop fought, accumulating per layer and
        // 3.3x heavier at 100k width. Free the games the cut keeps
        // nothing of; parent/act survive for reconstruction.
        for &n in &next[width.min(next.len())..] {
            arena[n].game = None;
        }
        next.truncate(width);
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod dbg_trace {
    use super::*;
    use crate::level::from_grid;

    #[test]
    #[ignore = "trace"]
    fn dbg_routes() {
        let cases: [(&str, &[&str], &str); 10] = [
            (
                "Kindling",
                &[
                    "#########",
                    "#@..y.g.#",
                    "#.#####.#",
                    "#..r..G.#",
                    "#.#####.#",
                    "#W.R..Y.#",
                    "#########",
                ],
                "rrrrrlllllddrrrxrrrddlllllluuddrl",
            ),
            (
                "Cupboard",
                &[
                    "#########",
                    "#..R.WY.#",
                    "#.#####.#",
                    "#@r...g.#",
                    "#.#####.#",
                    "#..y..G.#",
                    "#########",
                ],
                "rrrrrruuxllllllddddrrrrrruuduuull",
            ),
            (
                "Soot Gate",
                &[
                    "#########",
                    "#R#Y#W..#",
                    "#.#.#.###",
                    "#@.g.r.y#",
                    "#.#.#.#.#",
                    "#..G....#",
                    "#########",
                ],
                "rrrrlxrrrllxddlluUulldxUuUrrUrruu",
            ),
            (
                "Slow Burn",
                &[
                    "#########",
                    "#@....y.#",
                    "#######.#",
                    "#G....Y.#",
                    "#.#######",
                    "#..r.RgW#",
                    "#########",
                ],
                "rrrrrrddllllllddrrrrrrllllllrrrrrr",
            ),
            (
                "Pebble Gallery",
                &[
                    "###########",
                    "#@..r..Y..#",
                    "#.####.##.#",
                    "#.....G...#",
                    "#.####.##.#",
                    "#W..y.R.g.#",
                    "###########",
                ],
                "rrrrrdddxdllrrrrxruuuulllddddlllll",
            ),
            (
                "Hearth Row",
                &[
                    "###########",
                    "#W.R..Y..G#",
                    "#.###.##.##",
                    "#@...r....#",
                    "#.###.##.##",
                    "#.y...g...#",
                    "###########",
                ],
                "ddrluuuurrrrxddxUddrrruuuulxRllllll",
            ),
            (
                "Emberlight",
                &[
                    "###########",
                    "#W..R.Y.G.#",
                    "#.####.####",
                    "#@.......y#",
                    "#.####,####",
                    "#...r.g...#",
                    "###########",
                ],
                "rrrrrddllrruuuulxxRrddrrrllluuRlllll",
            ),
            (
                "Tinder Row",
                &[
                    "###########",
                    "#@.r..y..g#",
                    "#.#######.#",
                    "#...G.R.Y.#",
                    "#.#######.#",
                    "#.......W.#",
                    "###########",
                ],
                "ddrrrrrrrruuddxLuullllllllddrrrrrrrrddl",
            ),
            (
                "Ashfall",
                &[
                    "###########",
                    "#W.R.Y.G.B#",
                    "#.####.####",
                    "#@.....y.b#",
                    "#.####.####",
                    "#..r..g...#",
                    "###########",
                ],
                "ddrrrrrllllluurrrxrrrrrllluulllxlxxlddRrRrrrruxulxRrRdduurRllxRllll",
            ),
            (
                "Ninefold",
                &[
                    "#########",
                    "#W.....r#",
                    "#.#####.#",
                    "#.#YgG#.#",
                    "#.#.y.#.#",
                    "#.##R.#.#",
                    "#@......#",
                    "#########",
                ],
                "rrrrrruuuuudddddllluuurddlulurdddllluuuuu",
            ),
        ];
        for (name, rows, gl) in cases {
            let level = from_grid(name, rows, 0).expect("valid grid");
            let route = parse(gl);
            let mut g = Game::start(level.clone());
            println!("== {name} ==");
            let mut dead = false;
            for (n, a) in route.iter().enumerate() {
                let before_trail = g.trail.len();
                let before_locks = g
                    .ants
                    .iter()
                    .filter(|a| matches!(a.state, AntState::Asleep(_)))
                    .count();
                let r = match a {
                    Act::Move(d) => g.move_player(*d),
                    Act::Push(d) => g.push(*d),
                    Act::Detach => g.detach(),
                };
                if let Err(msg) = r {
                    println!("{name}: ILLEGAL at move {n} ({a:?}): {msg}");
                    dead = true;
                    break;
                }
                let c = g.player;
                if g.trail.len() > before_trail {
                    let id = *g.trail.last().unwrap();
                    println!(
                        "  m{n}: collect {} at ({},{})",
                        "rygb".chars().nth(id).unwrap(),
                        c.col,
                        c.row
                    );
                } else if g.trail.len() < before_trail {
                    println!("  m{n}: park tail at ({},{})", c.col, c.row);
                }
                let asleep: Vec<(usize, (i32, i32))> = g
                    .ants
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| matches!(a.state, AntState::Asleep(_)))
                    .map(|(i, a)| match a.state {
                        AntState::Asleep(cc) => (i, (cc.col, cc.row)),
                        _ => (i, (0, 0)),
                    })
                    .collect();
                if asleep.len() > before_locks {
                    let (id, pos) = *asleep.last().unwrap();
                    println!(
                        "  m{n}: {} ASLEEP on bed at ({},{})",
                        "rygb".chars().nth(id).unwrap(),
                        pos.0,
                        pos.1
                    );
                }
                if matches!(a, Act::Push(_)) && asleep.len() == before_locks {
                    println!("  m{n}: shove at ({},{})", c.col, c.row);
                }
                if g.won {
                    println!("  m{n}: WIN at ({},{})", c.col, c.row);
                    break;
                }
            }
            if !dead && !g.won {
                let asleep: Vec<usize> = g
                    .ants
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| matches!(a.state, AntState::Asleep(_)))
                    .map(|(i, _)| i)
                    .collect();
                println!(
                    "{name}: end not won asleep={asleep:?} trail={} player=({},{})",
                    g.trail.len(),
                    g.player.col,
                    g.player.row,
                );
            }
        }
    }

    /// Per-move narration of the two routes this session retook
    /// (Ninefold, Ashfall), so the walkthrough prose is written from
    /// what the game actually does, not a reading of the glyph string.
    /// Traces the SHIPPED grids at their SHIPPED par route.
    #[test]
    #[ignore = "narrate changed routes"]
    fn narrate_changed_routes() {
        let glyphs = [
            (
                "Winter Quarters",
                "rrrrrrddrrrruxDullllddllllllxrrrrrrddrrrruduuuu",
            ),
            (
                "Ember Deep",
                "rrrrrlllxrlrrrrrrrrrrddlllllllllllluuuurrrrrrrrrrrrddddl",
            ),
        ];
        for (name, gl) in glyphs {
            let level = crate::level::load_levels(crate::LEVELS)
                .into_iter()
                .find(|l| l.name == name)
                .expect("shipped hill");
            let mut g = Game::start(level.clone());
            println!(
                "== {name} par {} ==\nstart player=({},{})",
                level.par, g.player.col, g.player.row
            );
            for (n, act) in parse(gl).into_iter().enumerate() {
                let ok = apply(&mut g, act);
                let note = g.take_note().unwrap_or("");
                let asleep = g.ants_asleep();
                println!(
                    "  m{n:>2} {} ok={ok} player=({},{}) asleep={asleep} trail={}  {note}",
                    act.glyph(),
                    g.player.col,
                    g.player.row,
                    g.trail.len(),
                );
                if g.won {
                    println!("  m{n}: WIN");
                    break;
                }
            }
            assert!(verify(&level, &parse(gl)), "{name}: route must replay");
        }
    }
}
