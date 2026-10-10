//! The garden's numbers, as data: read at boot, watched while the
//! app runs, and swapped in from the file's new root the frame it
//! changes. Every field's default is the const in `main.rs` that
//! explains why that number is the number — the file states the
//! shipped pace, and the game re-reads what the file says.

use frost::ron::{Kind, Val};

/// The watched file's path, beside the shipped table entry of the
/// same name: development sees the file, a shipped binary sees only
/// the table — and watches a file that never answers.
pub(crate) const GAMEPLAY_NAME: &str = "gameplay";
pub(crate) const GAMEPLAY_FILE: &str = "assets/game/gameplay.ron";

/// Everything the process asks the data for. A new number joins by
/// name: a field here, a default below, a line in the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Config {
    /// Growth runs at `1 / grow_slowdown` of real time.
    pub(crate) grow_slowdown: f32,
    /// The water reserve drains over this many growth-clock seconds.
    pub(crate) drain_time: f32,
    /// The real seconds a dry plant's withering takes to yellow.
    pub(crate) dry_yellow_time: f32,
    /// The real seconds a watered plant takes to rewhiten.
    pub(crate) dry_white_time: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            grow_slowdown: crate::GROW_SLOWDOWN,
            drain_time: crate::DRAIN_TIME,
            dry_yellow_time: crate::DRY_YELLOW_TIME,
            dry_white_time: crate::DRY_WHITE_TIME,
        }
    }
}

impl Config {
    /// The config a root states: every field it names, every field
    /// it does not at its default. A field of the wrong shape — a
    /// word where a number stands — falls back where it lies and is
    /// named for the log: the garden grows as well as the file
    /// explains itself.
    /// The config a store answers: bytes for `gameplay`, parsed,
    /// read. Every refusal keeps the defaults standing and names
    /// itself in the log — the garden starts honest or not at all,
    /// and never over a silent miss.
    pub(crate) fn from_store(store: &frost::Assets) -> Self {
        let Some(bytes) = store.bytes(GAMEPLAY_NAME) else {
            log::warn!("config: the store answers nothing for '{GAMEPLAY_NAME}' — defaults stand");
            return Self::default();
        };
        match frost::ron::parse(&String::from_utf8_lossy(&bytes)) {
            Ok(root) => {
                let (config, refused) = Self::read(&root);
                for why in &refused {
                    log::warn!("config: {why}");
                }
                config
            }
            Err(err) => {
                log::warn!("config: '{GAMEPLAY_NAME}' is not RON ({err}) — defaults stand");
                Self::default()
            }
        }
    }

    pub(crate) fn read(root: &Val) -> (Self, Vec<String>) {
        let mut config = Self::default();
        let mut refused = Vec::new();
        let mut number = |key: &str, slot: &mut f32| {
            let found = match root {
                Val::Struct { fields, .. } => {
                    fields
                        .iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, item)| match &item.val {
                            Val::Atom(text, Kind::Num) => {
                                text.parse::<f32>().ok().filter(|v| *v > 0.0)
                            }
                            _ => None,
                        })
                }
                _ => None,
            };
            match found {
                Some(Some(value)) => *slot = value,
                Some(None) => refused.push(format!("'{key}' is not a positive number")),
                None => refused.push(format!("'{key}' is not in the file")),
            }
        };
        number("grow_slowdown", &mut config.grow_slowdown);
        number("drain_time", &mut config.drain_time);
        number("dry_yellow_time", &mut config.dry_yellow_time);
        number("dry_white_time", &mut config.dry_white_time);
        (config, refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped file states exactly the shipped defaults: the
    /// data and the code agree, and this test is where they are
    /// made to say so.
    #[test]
    fn the_shipped_file_states_the_shipped_pace() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/game/gameplay.ron"
        ))
        .expect("the gameplay file rides the repository");
        let root = frost::ron::parse(&src).expect("the shipped file parses");
        let (config, refused) = Config::read(&root);
        assert!(refused.is_empty(), "refused: {refused:?}");
        assert_eq!(config, Config::default());
    }

    /// A file reads as far as it is honest: a good field joins, a
    /// word where a number stands is refused by name, and the
    /// field nobody wrote keeps its default.
    #[test]
    fn a_file_reads_as_far_as_it_is_honest() {
        let root = frost::ron::parse("(dry_white_time: 2.0, drain_time: \"two point seven five\")")
            .unwrap();
        let (config, refused) = Config::read(&root);
        assert_eq!(config.dry_white_time, 2.0);
        assert_eq!(config.drain_time, crate::DRAIN_TIME);
        assert_eq!(config.grow_slowdown, crate::GROW_SLOWDOWN);
        assert!(refused.iter().any(|r| r.contains("drain_time")));
        assert!(refused.iter().any(|r| r.contains("grow_slowdown")));
    }
}
