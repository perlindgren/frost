//! The game's save/reload: a versioned snapshot of the whole game state,
//! serialized with `serde` and stored as human-readable RON in
//! `~/.frost/immortal/snapshot.ron` — F5 writes it, F9 reloads the last
//! one, and the `--load` command line flag loads it at start up.
//!
//! The snapshot carries the random streams' states — the demo's jitter
//! source, the bug swarm's spawn randomizer, and the worms' spawn
//! randomizer — so a reloaded game continues the exact same random
//! streams, and the demo's clock, the tools and their poses, the plants
//! and their water reserves, the fallen fruit, the basket's fruit, the
//! bugs, the vipers, and the worms.
//!
//! The format is versioned: [VERSION] names the layout this build reads,
//! and [load] rejects a file written by any other version, so a future
//! layout can change without silently misreading an old file — bump
//! [VERSION] and, if the change is gentle, migrate in [parse] before the
//! check.

use serde::{Deserialize, Serialize};

use crate::{
    Fly, Pick, Tool, WateredPlant, bugs,
    fall::{Fall, FallPhase},
    plant, vipers, worms,
};

/// The snapshot format's version: the layout this build reads and writes.
/// [load] rejects files whose version differs.
pub const VERSION: u32 = 3;

/// A snapshot load's failure: the file is missing or unreadable, it is
/// not valid RON, or it was written by another format version.
#[derive(Debug)]
pub struct LoadError(String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

/// The whole game's state, as of one moment: [crate::Demo] writes it on
/// F5, and reads it back on F9 or at start up with `--load`.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    /// The [VERSION] the file was written with; [load] rejects mismatches.
    pub version: u32,
    /// The demo's random stream's state — the particle jitter's source.
    pub seed: u64,
    /// The bug swarm's random stream's state — the spawn and wander source.
    pub bug_seed: u64,
    /// The demo's running clock, in real seconds, from the game's start.
    pub time: f32,
    /// The particle emission accumulator.
    pub acc: f32,
    /// Whether the game-over overlay is up.
    pub over: bool,
    /// Whether the player has ever planted a seed.
    pub ever_planted: bool,
    /// The tools resting in the items panel's slots, in slot order.
    pub slots: [Option<Tool>; crate::SLOTS],
    /// The tool the mouse actively holds, if any.
    pub active: Option<Tool>,
    /// The stored tool the mouse carries in its other hand, if any.
    pub held: Option<Tool>,
    /// The tomato the mouse is carrying, if any.
    pub picking: Option<PickState>,
    /// The seed tomato in flight, if any.
    pub flying: Option<FlyState>,
    /// The carried fruit's body tint — the modulate frozen on the moment
    /// it was picked off the plant, ripe red toward the overgrown dark —
    /// and `None` while the mouse carries no fruit.
    pub held_tint: Option<[f32; 4]>,
    /// The active tool's live tilt, in radians.
    pub angle: f32,
    /// The active tool's tilt tween, so an ongoing tilt, return, or burst
    /// resumes mid-travel.
    pub turn: TurnState,
    /// The spray can's burst's elapsed time, in seconds, while one is in
    /// flight.
    pub burst: Option<f32>,
    /// Whether the spray can shows its burst frame.
    pub showing_spray2: bool,
    /// Whether the pour loop plays.
    pub pouring: bool,
    /// Whether the basket is still waiting to seed its starting tomato.
    pub basket_seed: bool,
    /// The basket's fruit, in container order — each fruit's body center,
    /// in the basket's local space, so a window resize moves none of it.
    pub basket_fruit: Vec<[f32; 2]>,
    /// The plants and their water reserves, in slot order.
    pub plants: [WateredPlantState; crate::PLANT_POS.len()],
    /// The fallen overgrown tomatoes, in drop order.
    pub falls: Vec<FallState>,
    /// The bug swarm's state.
    pub bugs: bugs::BugsState,
    /// The vipers' swarm state.
    pub vipers: vipers::VipersState,
    /// The worms' swarm state.
    pub worms: worms::WormsState,
}

/// A picked tomato, as of a snapshot: a plant fruit, by its plant and
/// bloom slot, or a basket fruit, by the pivot's origin in the basket's
/// local space — the spot it was picked from, where a release that does
/// not plant flies back to it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum PickState {
    /// Ripe fruit picked off plant `0`'s bloom slot `1`.
    Plant(usize, usize),
    /// A tomato picked up out of the basket: its pivot's origin in the
    /// basket's local space.
    Basket([f32; 2]),
}

impl From<Pick> for PickState {
    fn from(pick: Pick) -> Self {
        match pick {
            Pick::Plant(pi, si) => PickState::Plant(pi, si),
            Pick::Basket(origin) => {
                let [x, y] = origin.apply([0.0, 0.0]);
                PickState::Basket([x, y])
            }
        }
    }
}

impl From<PickState> for Pick {
    fn from(state: PickState) -> Self {
        match state {
            PickState::Plant(pi, si) => Pick::Plant(pi, si),
            PickState::Basket(origin) => Pick::Basket(frost::Transform::translate(origin)),
        }
    }
}

/// A seed's flight, as of a snapshot: the body-center tween's ends and
/// the flight's elapsed time, and the landing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FlyState {
    /// The body center the flight starts from, in user space.
    pub body: [f32; 2],
    /// The body center the flight lands on, in user space.
    pub dest: [f32; 2],
    /// The flight's elapsed time, in real seconds, since the drop.
    pub time: f32,
    /// Where the flight lands.
    pub landing: LandingState,
}

impl From<&Fly> for FlyState {
    fn from(fly: &Fly) -> Self {
        FlyState {
            body: fly.tween.from(),
            dest: fly.tween.to(),
            time: fly.time,
            landing: (&fly.landing).into(),
        }
    }
}

impl From<FlyState> for Fly {
    fn from(state: FlyState) -> Self {
        Fly {
            tween: frost::Tween::new(state.body, state.dest, crate::FLY_TIME)
                .repeat(frost::Repeat::Once),
            time: state.time,
            landing: state.landing.into(),
        }
    }
}

/// A seed's landing, as of a snapshot.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum LandingState {
    /// The seed plants into this slot on arrival.
    Slot(usize),
    /// Every slot is planted: the fruit flies back to the basket at this
    /// pivot origin, in the basket's local space.
    Basket([f32; 2]),
}

impl From<&crate::Landing> for LandingState {
    fn from(landing: &crate::Landing) -> Self {
        match landing {
            crate::Landing::Slot(i) => LandingState::Slot(*i),
            crate::Landing::Basket(origin) => LandingState::Basket(origin.apply([0.0, 0.0])),
        }
    }
}

impl From<LandingState> for crate::Landing {
    fn from(state: LandingState) -> Self {
        match state {
            LandingState::Slot(i) => crate::Landing::Slot(i),
            LandingState::Basket(origin) => {
                crate::Landing::Basket(frost::Transform::translate(origin))
            }
        }
    }
}

/// A fallen tomato, as of a snapshot: every field of the [Fall].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct FallState {
    /// The phase the tomato is in.
    pub phase: FallPhase,
    /// Where the fall stops, in user space.
    pub land_pos: [f32; 2],
    /// Whether the drop clip has played for this fruit.
    pub landed: bool,
    /// The body center's position, in user space.
    pub body: [f32; 2],
    /// The target plant anchor, in user space.
    pub target: [f32; 2],
    /// The body position the tomato started its current roll from.
    pub start: [f32; 2],
    /// The roll progress, 0.0 (at `start`) to 1.0 (at `target`).
    pub progress: f32,
    /// The straight-line distance from `start` to `target`.
    pub distance: f32,
    /// The running bump's progress, or [`crate::fall::BUMP_NONE`] while none runs.
    pub bump: f32,
    /// The seconds left before the next bump arms.
    pub bump_in: f32,
    /// The tomato's wheel rotation, in radians.
    pub spin: f32,
    /// The tomato's current roll speed, in user pixels per second.
    pub speed: f32,
    /// The seconds the tomato has spent resting at its target.
    pub rest: f32,
    /// How long this rest lasts, in seconds, before the next roll starts.
    pub rest_for: f32,
}

impl From<&Fall> for FallState {
    fn from(fall: &Fall) -> Self {
        FallState {
            phase: fall.phase,
            land_pos: fall.land_pos,
            landed: fall.landed,
            body: fall.body,
            target: fall.target,
            start: fall.start,
            progress: fall.progress,
            distance: fall.distance,
            bump: fall.bump,
            bump_in: fall.bump_in,
            spin: fall.spin,
            speed: fall.speed,
            rest: fall.rest,
            rest_for: fall.rest_for,
        }
    }
}

impl From<FallState> for Fall {
    fn from(state: FallState) -> Self {
        Fall {
            phase: state.phase,
            land_pos: state.land_pos,
            landed: state.landed,
            body: state.body,
            target: state.target,
            start: state.start,
            progress: state.progress,
            distance: state.distance,
            bump: state.bump,
            bump_in: state.bump_in,
            spin: state.spin,
            speed: state.speed,
            rest: state.rest,
            rest_for: state.rest_for,
        }
    }
}

/// A watered plant, as of a snapshot: its water and dryness, and its
/// plant's state.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WateredPlantState {
    /// Whether a seed has landed in this slot and its plant is growing.
    pub planted: bool,
    /// The water reserve, 1.0 (well watered) to 0.0 (dry).
    pub water: f32,
    /// The dryness, 0.0 (white — watered) to 1.0 (full yellow — withered).
    pub dryness: f32,
    /// The plant's growth state.
    pub plant: plant::PlantState,
}

impl From<&WateredPlant> for WateredPlantState {
    fn from(wp: &WateredPlant) -> Self {
        WateredPlantState {
            planted: wp.planted,
            water: wp.water,
            dryness: wp.dryness,
            plant: plant::PlantState::from(&wp.plant),
        }
    }
}

/// The active tool's tilt tween, captured in full — its ends, its
/// elapsed time, its leg duration, and its repeat mode — so an ongoing
/// tilt, return, or burst resumes exactly where it was.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct TurnState {
    /// The angle the tween starts from, in radians.
    pub from: f32,
    /// The angle the tween travels to, in radians.
    pub to: f32,
    /// The elapsed time, in seconds, since the tween started.
    pub time: f32,
    /// Seconds for one leg (the `from -> to` travel).
    pub duration: f32,
    /// The repeat mode.
    pub repeat: RepeatState,
}

/// The repeat mode of a [TurnState].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatState {
    /// Travel to the target and stay there.
    Once,
    /// Jump back to the start and travel again.
    Loop,
    /// Travel to the target, then back to the start, forever.
    PingPong,
}

impl From<frost::Repeat> for RepeatState {
    fn from(repeat: frost::Repeat) -> Self {
        match repeat {
            frost::Repeat::Once => RepeatState::Once,
            frost::Repeat::Loop => RepeatState::Loop,
            frost::Repeat::PingPong => RepeatState::PingPong,
        }
    }
}

impl From<RepeatState> for frost::Repeat {
    fn from(state: RepeatState) -> Self {
        match state {
            RepeatState::Once => frost::Repeat::Once,
            RepeatState::Loop => frost::Repeat::Loop,
            RepeatState::PingPong => frost::Repeat::PingPong,
        }
    }
}

impl From<&frost::Tween<f32>> for TurnState {
    fn from(tween: &frost::Tween<f32>) -> Self {
        TurnState {
            from: tween.from(),
            to: tween.to(),
            time: tween.time(),
            duration: tween.duration(),
            repeat: tween.repeat_mode().into(),
        }
    }
}

impl TurnState {
    /// The tween this state restores: built fresh, then clocked back to
    /// its captured moment.
    pub fn into_tween(self) -> frost::Tween<f32> {
        let mut tween =
            frost::Tween::new(self.from, self.to, self.duration).repeat(self.repeat.into());
        tween.set_time(self.time);
        tween
    }
}

/// The snapshot's file: `~/.frost/immortal/snapshot.ron` — the user's
/// home directory (`USERPROFILE` on Windows), one folder per game, so the
/// snapshot survives the working directory changing. Without a home
/// directory it falls back to a plain `snapshot.ron` in the working
/// directory.
pub fn snapshot_path() -> std::path::PathBuf {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    };
    match home {
        Ok(home) => std::path::PathBuf::from(home)
            .join(".frost")
            .join("immortal")
            .join("snapshot.ron"),
        Err(_) => std::path::PathBuf::from("snapshot.ron"),
    }
}

/// Writes `snapshot` to [snapshot_path], creating the folder if needed.
pub fn save(snapshot: &Snapshot) -> std::io::Result<()> {
    let path = snapshot_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = ron::to_string(snapshot)
        .map_err(|err| std::io::Error::other(format!("encoding the snapshot: {err}")))?;
    std::fs::write(&path, text)
}

/// Reads and parses the text of a snapshot file, checking its version:
/// a file written by another [VERSION] is rejected, so a future layout
/// can change without silently misreading an old file.
pub fn parse(text: &str) -> Result<Snapshot, LoadError> {
    let snapshot: Snapshot = ron::from_str(text)
        .map_err(|err| LoadError(format!("the snapshot is not valid RON: {err}")))?;
    if snapshot.version != VERSION {
        return Err(LoadError(format!(
            "the snapshot is version {} and this build reads version {VERSION}",
            snapshot.version
        )));
    }
    Ok(snapshot)
}

/// Reads the last snapshot from [snapshot_path] (see [parse] for the
/// version check).
pub fn load() -> Result<Snapshot, LoadError> {
    let path = snapshot_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|err| LoadError(format!("no snapshot to load at {}: {err}", path.display())))?;
    parse(&text).map_err(|err| {
        LoadError(format!(
            "the snapshot at {} cannot be loaded: {err}",
            path.display()
        ))
    })
}
