//! Pure game logic for Ember Hill — no rendering, no engine types.
//!
//! The trail is a sliding chain packed along the player's path history:
//! `history` is a deque whose front is the player's current cell. A move
//! onto the cell directly behind the player pops the front (a U-turn
//! slides the packed line), every other move pushes a new front. Trail
//! ant `i` (0 = head, collected first, nearest the player) sits at
//! `history[min(i + 1, len - 1)]` — when the history runs out, ants
//! *ride* at the player's cell. Collecting appends to the tail, and the
//! tail may be left behind (detach) to shuffle the stack.
//!
//! **Lock-in is automatic**: whenever the *tail* ant of the line — the
//! last one collected — comes to rest on its own bed, walking there in
//! the player's wake or riding on it, it falls asleep there. No button
//! puts anyone to bed. Only the tail can leave the line, so a full line
//! peels off last-in-first-warm: to bed ants in a chosen order, collect
//! them backwards and shuffle with the leave-behind. Leaving-behind is
//! the manual twin of lock-in: same tail ant, dropped where it stands
//! instead of where it sleeps.
//!
//! A bed with a sleeper in it is a blocker: nobody walks, pushes, or
//! packs onto it, so each delivery reshapes the hill. Two corollaries
//! of the U-turn pop follow: a dead-end cell is never a rest cell (its
//! bed can only be filled by *pushing* an ant into it), and once a bed
//! warms behind you, the line must route around the new wall.

use crate::level::{Level, Tile};
use std::collections::VecDeque;

/// History longer than this is pruned at the back; no trail is that deep.
const MAX_HISTORY: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Cell {
    pub col: i32,
    pub row: i32,
}

impl Cell {
    pub fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }
    pub fn at(self, dir: Dir) -> Cell {
        let (dc, dr) = match dir {
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
            Dir::Up => (0, -1), // row 0 is the top row; up = row - 1
            Dir::Down => (0, 1),
        };
        Cell {
            col: self.col + dc,
            row: self.row + dr,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AntColor {
    White, // the player's color
    Red,
    Yellow,
    Green,
    Blue,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AntState {
    Wild(Cell),
    Trail,
    Placed(Cell),
    Asleep(Cell),
}

#[derive(Clone, Copy, Debug)]
pub struct Ant {
    pub color: AntColor,
    pub state: AntState,
}

/// One occupant of a cell that is not the player and not a trail ant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Occ {
    Wild(usize),
    Placed(usize),
    Asleep(usize),
}

#[derive(Clone)]
struct Snap {
    tiles: Vec<Tile>,
    player: Cell,
    history: VecDeque<Cell>,
    trail: Vec<usize>,
    ants: Vec<Ant>,
    moves: i32,
    won: bool,
}

#[derive(Clone)]
pub struct Game {
    pub level: Level,
    pub tiles: Vec<Tile>,
    pub player: Cell,
    pub history: VecDeque<Cell>,
    /// Ant ids, head first. The head is nearest the player, the *last*
    /// entry is the tail — the only ant detach can release.
    pub trail: Vec<usize>,
    pub ants: Vec<Ant>,
    pub moves: i32,
    pub won: bool,
    undo: Vec<Snap>,
    note: Option<&'static str>,
}

impl Game {
    pub fn new(level: Level) -> Self {
        let tiles = level.tiles.clone();
        let ants = level
            .wild
            .iter()
            .map(|(cell, color)| Ant {
                color: *color,
                state: AntState::Wild(*cell),
            })
            .collect();
        Self {
            level,
            tiles,
            player: Cell::new(0, 0),
            history: VecDeque::from([Cell::new(0, 0)]),
            trail: Vec::new(),
            ants,
            moves: 0,
            won: false,
            undo: Vec::new(),
            note: None,
        }
    }

    pub fn start(level: Level) -> Self {
        let player = level.player_start;
        let mut game = Self::new(level);
        game.player = player;
        game.history = VecDeque::from([player]);
        game
    }

    // ------------------------------------------------------------------
    // Board helpers
    // ------------------------------------------------------------------

    pub fn in_bounds(&self, c: Cell) -> bool {
        c.col >= 0 && c.col < self.level.cols as i32 && c.row >= 0 && c.row < self.level.rows as i32
    }

    pub fn tile(&self, c: Cell) -> Tile {
        if self.in_bounds(c) {
            self.tiles[(c.row * self.level.cols as i32 + c.col) as usize]
        } else {
            Tile::Rock
        }
    }

    pub fn walkable(&self, c: Cell) -> bool {
        matches!(self.tile(c), Tile::Tunnel | Tile::Bed(_))
    }

    /// The cell trail ant `i` currently occupies (history-clamped).
    pub fn trail_target(&self, i: usize) -> Cell {
        let idx = (i + 1).min(self.history.len() - 1);
        self.history
            .get(idx)
            .copied()
            .expect("history is never empty")
    }

    pub fn trail_cell_occupied(&self, c: Cell) -> bool {
        (0..self.trail.len()).any(|i| self.trail_target(i) == c)
    }

    /// An ant standing at `c` that is neither the player nor a trail ant.
    pub fn occupant(&self, c: Cell) -> Option<Occ> {
        self.ants
            .iter()
            .enumerate()
            .find_map(|(id, ant)| match ant.state {
                AntState::Wild(p) if p == c => Some(Occ::Wild(id)),
                AntState::Placed(p) if p == c => Some(Occ::Placed(id)),
                AntState::Asleep(p) if p == c => Some(Occ::Asleep(id)),
                _ => None,
            })
    }

    pub fn ants_asleep(&self) -> usize {
        self.ants
            .iter()
            .filter(|a| matches!(a.state, AntState::Asleep(_)))
            .count()
    }

    pub fn take_note(&mut self) -> Option<&'static str> {
        self.note.take()
    }

    /// The solver expands through the game thousands of times; snapshots
    /// are only ever needed for player-facing undo, so the search clears
    /// them after every applied action. Test-only: the solver is the
    /// sole caller.
    #[cfg(test)]
    pub(crate) fn forget_history(&mut self) {
        self.undo.clear();
    }

    fn snap(&self) -> Snap {
        Snap {
            tiles: self.tiles.clone(),
            player: self.player,
            history: self.history.clone(),
            trail: self.trail.clone(),
            ants: self.ants.clone(),
            moves: self.moves,
            won: self.won,
        }
    }

    fn restore(&mut self, s: Snap) {
        self.tiles = s.tiles;
        self.player = s.player;
        self.history = s.history;
        self.trail = s.trail;
        self.ants = s.ants;
        self.moves = s.moves;
        self.won = s.won;
    }

    fn push_undo(&mut self) {
        let s = self.snap();
        self.undo.push(s);
    }

    /// Undo one action. Returns false when nothing is left to undo.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(s) => {
                self.restore(s);
                self.note = None;
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    /// Plain move: walk, collect, dig & move, or chain-shift on a U-turn.
    pub fn move_player(&mut self, dir: Dir) -> Result<(), &'static str> {
        let d = self.player.at(dir);
        if self.won {
            return Err("the hill sleeps");
        }
        if !self.in_bounds(d) || self.tile(d) == Tile::Rock {
            return Err("solid rock");
        }
        // Own line in the way?
        if self.trail_cell_occupied(d) {
            if self.history.get(1) == Some(&d) {
                return self.u_turn();
            }
            return Err("that is your own line");
        }
        // Occupants.
        if let Some(occ) = self.occupant(d) {
            return match occ {
                Occ::Wild(id) | Occ::Placed(id) => self.collect(d, id),
                Occ::Asleep(_) => Err("someone is already warm there"),
            };
        }
        match self.tile(d) {
            Tile::Dirt => self.dig_and_move(d),
            Tile::Tunnel | Tile::Bed(_) => {
                self.push_undo();
                self.walk(d);
                self.after_action("the line has nowhere to pack")
            }
            Tile::Rock => unreachable!("checked above"),
        }
    }

    /// Shift+move: shove the first placed/wild ant one cell. A matching
    /// bed at the push target locks it in — the dunk shot, and the only
    /// way to warm a bed at the end of a dead-end corridor.
    pub fn push(&mut self, dir: Dir) -> Result<(), &'static str> {
        if self.won {
            return Err("the hill sleeps");
        }
        let d = self.player.at(dir);
        let Some(occ) = self.occupant(d) else {
            if self.in_bounds(d) && self.trail_cell_occupied(d) {
                return Err("you cannot push your own line");
            }
            return Err("nothing to push");
        };
        let id = match occ {
            Occ::Wild(id) | Occ::Placed(id) => id,
            Occ::Asleep(_) => return Err("let sleeping ants lie"),
        };
        let d2 = d.at(dir);
        if !self.walkable(d2) {
            return Err("nowhere to push it");
        }
        if d2 == self.player || self.occupant(d2).is_some() || self.trail_cell_occupied(d2) {
            return Err("nowhere to push it");
        }
        self.push_undo();
        self.ants[id].state = if self.tile(d2) == Tile::Bed(self.ants[id].color) {
            self.note = Some(warm_note(self.ants[id].color));
            AntState::Asleep(d2)
        } else {
            match self.ants[id].state {
                AntState::Wild(_) => AntState::Wild(d2),
                _ => AntState::Placed(d2),
            }
        };
        self.moves += 1;
        self.check_win();
        Ok(())
    }

    /// Space: leave the last ant of the line behind, at the cell it
    /// rests on, so the stack can be shuffled. Riding ants stay aboard;
    /// a bunched line must stretch first. Nobody can be put to bed by
    /// hand anymore — lock-in is automatic.
    pub fn detach(&mut self) -> Result<(), &'static str> {
        if self.won {
            return Err("the hill sleeps");
        }
        let tail_i = match self.trail.len() {
            0 => return Err("nobody is following"),
            n => n - 1,
        };
        let id = self.trail[tail_i];
        let tail_cell = self.trail_target(tail_i);
        if tail_cell == self.player {
            return Err("an ant riding at the back stays aboard");
        }
        if tail_i > 0 && self.trail_target(tail_i - 1) == tail_cell {
            return Err("the line is bunched  stretch it first");
        }
        self.push_undo();
        self.ants[id].state = AntState::Placed(tail_cell);
        self.trail.pop();
        self.moves += 1;
        self.check_win();
        Ok(())
    }

    fn u_turn(&mut self) -> Result<(), &'static str> {
        let d = self.history.get(1).copied().expect("checked by caller");
        self.push_undo();
        self.history.pop_front();
        self.player = d;
        self.moves += 1;
        self.after_action("the line has nowhere to pack")
    }

    fn collect(&mut self, d: Cell, id: usize) -> Result<(), &'static str> {
        self.push_undo();
        self.walk(d);
        self.ants[id].state = AntState::Trail;
        self.trail.push(id);
        self.after_action("the line has nowhere to pack")
    }

    fn dig_and_move(&mut self, d: Cell) -> Result<(), &'static str> {
        self.push_undo();
        self.tiles[(d.row * self.level.cols as i32 + d.col) as usize] = Tile::Tunnel;
        self.walk(d);
        self.after_action("the line has nowhere to pack")
    }

    /// Move the player and slide the history: U-turn pops, else push.
    fn walk(&mut self, d: Cell) {
        if self.history.len() >= 2 && self.history.get(1) == Some(&d) {
            self.history.pop_front();
        } else {
            self.history.push_front(d);
            if self.history.len() > MAX_HISTORY {
                self.history.pop_back();
            }
        }
        self.player = d;
        self.moves += 1;
    }

    /// Validate the trail after an action: first every ant that came to
    /// rest on its own bed locks in (possibly cascading, as the line
    /// shifts forward), then the resting spots must all be free. On
    /// conflict, roll back and explain.
    fn after_action(&mut self, why: &'static str) -> Result<(), &'static str> {
        self.resolve_locks();
        let clash = (0..self.trail.len()).any(|i| self.occupant(self.trail_target(i)).is_some());
        if clash {
            let s = self.undo.pop().expect("push_undo ran first");
            self.restore(s);
            return Err(why);
        }
        self.check_win();
        Ok(())
    }

    /// Auto-lock: the tail ant resting on its own bed falls asleep
    /// there. Only the tail peels, so nobody crumples into a fresh bed;
    /// the loop covers a bunched (riding) line leaving two ants on the
    /// same cell one after the other.
    fn resolve_locks(&mut self) {
        while let Some(&id) = self.trail.last() {
            let cell = self.trail_target(self.trail.len() - 1);
            if cell != self.bed_of(self.ants[id].color) {
                break;
            }
            self.ants[id].state = AntState::Asleep(cell);
            self.trail.pop();
            self.note = Some(warm_note(self.ants[id].color));
        }
    }

    /// The dedicated bed cell of `color` in this hill.
    fn bed_of(&self, color: AntColor) -> Cell {
        self.level
            .beds
            .iter()
            .find(|&&(_, c)| c == color)
            .map(|&(cell, _)| cell)
            .expect("every ant color has a bed (level validator)")
    }

    fn check_win(&mut self) {
        self.won = self
            .ants
            .iter()
            .all(|a| matches!(a.state, AntState::Asleep(_)))
            && self.tile(self.player) == Tile::Bed(AntColor::White);
    }
}

fn warm_note(color: AntColor) -> &'static str {
    match color {
        AntColor::White => "the queen is warm",
        AntColor::Red => "red is warm",
        AntColor::Yellow => "yellow is warm",
        AntColor::Green => "green is warm",
        AntColor::Blue => "blue is warm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::from_grid;

    /// The new tutorial ring: `r` warms in the player's wake as they pass
    /// the bed, and the warm bed then walls off the top corridor — home
    /// is the other way round.
    const FIRST_COLD: [&str; 5] = [
        "###########",
        "#W..r@.R..#",
        "#.#######.#",
        "#.........#",
        "###########",
    ];

    fn game(rows: &[&str]) -> Game {
        Game::start(from_grid("test", rows, 0).expect("valid test level"))
    }

    fn walk(g: &mut Game, dirs: &[Dir]) {
        for d in dirs {
            g.move_player(*d).expect("legal walk");
        }
    }

    #[test]
    fn wake_lock_warms_r_and_the_ring_carries_the_win() {
        let mut g = game(&FIRST_COLD);
        g.move_player(Dir::Left).expect("collect r");
        assert_eq!(g.trail.len(), 1);
        walk(&mut g, &[Dir::Right; 3]); // across the bed: r one behind, still in line
        assert_eq!(g.ants_asleep(), 0, "not resting on it yet");
        g.move_player(Dir::Right).expect("step past the bed");
        assert_eq!(g.ants_asleep(), 1, "r landed on R and locked in");
        assert!(g.trail.is_empty());
        assert_eq!(
            g.move_player(Dir::Left),
            Err("someone is already warm there"),
            "a warmed bed cannot be passed"
        );
        g.move_player(Dir::Right).expect("to the far shaft");
        walk(&mut g, &[Dir::Down; 2]); // down to the bottom corridor
        walk(&mut g, &[Dir::Left; 8]);
        walk(&mut g, &[Dir::Up; 2]); // up the left shaft to W
        assert!(g.won, "every ant is warm and the player is home");
        assert_eq!(g.moves, 18);
    }

    #[test]
    fn a_dead_end_bed_never_gets_a_resting_trail_ant() {
        // Leaving a dead end is always a U-turn pop: the end cell never
        // re-enters history at an index an ant rests on. End beds are
        // push-only — the rule that reshapes every hill.
        let rows = ["#########", "#W..r@.R#", "#########"];
        let mut g = game(&rows);
        walk(&mut g, &[Dir::Left; 3]); // collect r
        walk(&mut g, &[Dir::Right; 5]); // stand on the bed itself
        assert_eq!(g.tile(g.player), Tile::Bed(AntColor::Red));
        assert_eq!(g.ants_asleep(), 0, "riding is not resting");
    }

    #[test]
    fn dunk_push_locks_and_the_bed_turns_to_wall() {
        let rows = ["########", "#W@.rR.#", "########"];
        let mut g = game(&rows);
        g.move_player(Dir::Right).expect("step beside r");
        g.push(Dir::Right).expect("dunk r into its bed");
        assert!(
            matches!(
                g.ants[0].state,
                AntState::Asleep(c) if c == Cell::new(5, 1)
            ),
            "the push locks r on R"
        );
        g.move_player(Dir::Right).expect("step into r's old cell");
        assert_eq!(
            g.move_player(Dir::Right),
            Err("someone is already warm there")
        );
    }

    #[test]
    fn leave_the_last_ant_behind_and_take_it_back() {
        let mut g = game(&FIRST_COLD);
        g.move_player(Dir::Left).expect("collect r");
        g.move_player(Dir::Right)
            .expect("retrace: the line packs in");
        assert_eq!(
            g.detach(),
            Err("an ant riding at the back stays aboard"),
            "a packed line drops nobody"
        );
        g.move_player(Dir::Right).expect("stretch the line"); // r rests at the start cell
        g.detach().expect("leave r behind");
        assert!(matches!(
            g.ants[0].state,
            AntState::Placed(c) if c == Cell::new(5, 1)
        ));
        g.move_player(Dir::Left).expect("walk back onto r");
        assert_eq!(g.trail.len(), 1, "r falls in behind again");
    }

    #[test]
    fn dirt_gives_way_freely_and_the_line_follows() {
        let rows = ["########", "#W....B#", "#@,,,b,#", "########"];
        let mut g = game(&rows);
        walk(&mut g, &[Dir::Right; 5]); // four cells of dirt deep, b joined on the way
        assert_eq!(g.trail.len(), 1, "the stranded blue joined");
        assert_eq!(
            g.trail_target(0),
            Cell::new(5, 2),
            "the line sits one behind the dug end"
        );
        assert!(
            g.move_player(Dir::Right).is_err(),
            "only rock still says no"
        );
    }

    #[test]
    fn undo_rolls_back_one_action() {
        let mut g = game(&FIRST_COLD);
        g.move_player(Dir::Left).expect("one step");
        assert_eq!(g.moves, 1);
        assert!(g.undo());
        assert_eq!(g.moves, 0);
        assert_eq!(g.player, Cell::new(5, 1));
        assert!(!g.undo(), "nothing older to undo");
    }

    #[test]
    fn shipped_levels_all_parse() {
        let levels = crate::level::load_levels(crate::LEVELS);
        assert!(!levels.is_empty(), "the validator ate every hill");
    }
}
