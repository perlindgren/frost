//! Solver: breadth-first search over the *real* game state machine,
//! with parent links so the winning node reconstructs to an action path.
//!
//! Every expansion drives the actual [`Game`], so reaching the win flag
//! is close to proof — and the returned path is re-applied to a fresh
//! game before it leaves `solution()`, making the proof airtight. The
//! BFS depth of a win is the true optimum: the `par` the HUD shows.

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

pub fn bfs(level: &Level, max_states: usize) -> Run {
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
    let run = bfs(level, max_states);
    run.winner.map(|_| path(&run)).filter(|p| verify(level, p))
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
        let run = bfs(&level, 1_000_000);
        assert!(run.winner.is_none());
        assert!(!run.capped, "small hill should exhaust, not cap");
    }

    #[test]
    #[ignore = "full solve of every shipped hill"]
    fn solve_all_shipped() {
        for level in crate::level::load_levels(crate::LEVELS) {
            let name = level.name.clone();
            let t = std::time::Instant::now();
            let run = bfs(&level, 24_000_000);
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
                None => eprintln!("{name}: par {} = verified route (cap hit)", level.par),
            }
            eprintln!("  {:?}", t.elapsed());
        }
    }

    /// BFS caps out on four trail ants (the history key is honest, so the
    /// space is enormous); this hand route is the shipped par and is
    /// replay-verified against the real game. Blue peels mid-dig in the
    /// low gallery, yellow just past its bed in the same run, green on
    /// the lapped upper ring, and red last — in the middle corridor's
    /// wake, one step from the way home.
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
        let glyphs = "rrrrrrrrrrrrllllllllllllddrrrrrrrrrrrruuuullllllllllllddrrrrrrrrrrrrddl";
        let route = parse(&glyphs);
        assert_eq!(route.len(), 71);
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
        let glyphs = "rrrrrrrrrrxxddddxlllluuuurlddrxLlLlLlLrrrrrrxLuu";
        let route = parse(glyphs);
        assert_eq!(route.len(), 48);
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
        let glyphs = "rrrrrruuuuudddddllluuurddlulurdddllluuuuu";
        let route = parse(glyphs);
        assert_eq!(route.len(), 41);
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
        let glyphs = "ddrrrrrllllluurrrxrrrrrllluulllxlxxlddRrRrrrruxulxRrRdduurRllxRllll";
        let route = parse(glyphs);
        assert_eq!(route.len(), 67);
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
            let run = bfs(&level, cap);
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
}

/// Route hunter for hills BFS cannot settle (four-ant trails). Layered
/// beam search over the REAL state machine: expand a frontier one ply,
/// score children by depth + 8 x heuristic (each ant's distance to its
/// bed, plus a doubled pull toward W once the colony sleeps), keep the
/// best `width` of them. Two honest compromises:
///
/// - the within-layer dedup key DROPS the history deque. That is not
///   sound for optimality (history gates packing legality), but this
///   tool never claims optimality: every route it returns must pass
///   `verify` before it means anything. A dropped merge can only make
///   the hunter miss routes, never invent one.
/// - a finite width can miss solutions that route through bad-looking
///   middles. Widen the beam or try again; the answer is trusted only
///   via replay anyway.
pub fn hunt_route(level: &Level, max_depth: usize) -> Option<Vec<Act>> {
    use std::collections::HashSet;

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
    let h = |g: &Game| -> i32 {
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
    };
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
    let width = 30_000usize;
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
            let level = from_grid(name, &rows, 0).expect("valid grid");
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
}
