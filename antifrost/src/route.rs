//! The alphabet a best run is written in. One recorded run is one
//! string: lowercase letters walk, uppercase shove, `x` leaves an ant
//! behind — the very letters the walkthrough speaks, so a hill's best
//! run and its written solution read the same way. A replay is that
//! string walked back through the hill.
use crate::game;

/// The ledger's route alphabet, borrowed from the walkthrough:
/// `lrud` walks, `LRUD` shoves, `x` leaves an ant behind. One string
/// is one recorded best run, and one replay is that string walked
/// back through the hill.
pub fn glyph_of(a: game::Act) -> char {
    use game::{Act, Dir};
    match a {
        Act::Move(Dir::Left) => 'l',
        Act::Move(Dir::Right) => 'r',
        Act::Move(Dir::Up) => 'u',
        Act::Move(Dir::Down) => 'd',
        Act::Push(Dir::Left) => 'L',
        Act::Push(Dir::Right) => 'R',
        Act::Push(Dir::Up) => 'U',
        Act::Push(Dir::Down) => 'D',
        Act::Detach => 'x',
    }
}

pub fn encode_run(acts: &[game::Act]) -> String {
    acts.iter().map(|a| glyph_of(*a)).collect()
}

/// Every glyph or nothing: a recording with a stray letter in it is
/// not a route, and the ledger pretends it never heard of it.
pub fn decode_run(s: &str) -> Option<Vec<game::Act>> {
    use game::{Act, Dir};
    s.chars()
        .map(|c| {
            Some(match c {
                'l' => Act::Move(Dir::Left),
                'r' => Act::Move(Dir::Right),
                'u' => Act::Move(Dir::Up),
                'd' => Act::Move(Dir::Down),
                'L' => Act::Push(Dir::Left),
                'R' => Act::Push(Dir::Right),
                'U' => Act::Push(Dir::Up),
                'D' => Act::Push(Dir::Down),
                'x' => Act::Detach,
                _ => return None,
            })
        })
        .collect()
}

/// A recorded best run, walked back through the hill. The live game
/// *is* the showing's current frame — `shown` actions of `acts` have
/// been applied to it — so everything that paints a hill keeps
/// painting one; stepping back rebuilds the hill from its own
/// beginning through one fewer action.
pub struct Replay {
    pub acts: Vec<game::Act>,
    pub shown: usize,
    pub playing: bool,
}
