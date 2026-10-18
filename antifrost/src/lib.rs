//! Ember Hill's own logic, apart from any window: the hill itself
//! (`game`), its authored shape (`level`), the ledger of runs
//! (`ledger`), the player's dials (`settings`), and the alphabet a
//! run is written in (`route`). What a hill IS lives here; the
//! binary — the hill as played, painted, and shown — is beside it,
//! and the tests hold this half to its own laws.
pub mod game;
pub mod ledger;
pub mod level;
pub mod route;
pub mod settings;

/// Every hill the descent ships, as authored.
pub const LEVELS: &str = include_str!("../levels.ron");

/// The hunter and its certifier: a test-only half that proves every
/// shipped hill winnable and every recorded route honest. It searches
/// the domain, never the window, so it lives with the domain.
#[cfg(test)]
mod solver;
