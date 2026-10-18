//! Level data: an ASCII grid inside a RON document, parsed with the
//! engine's own `frost::ron` tree reader (the immortal/config.rs way).
//!
//! Legend:
//! - `#` rock (solid, forever)      - `,` dirt (diggable)
//! - `.` tunnel                      - `@` player start
//! - Uppercase `W R Y G B`: empty bed of that color (`W` = player's bed)
//! - lowercase `r y g b`: one wild ant of that color, born stranded
//!
//! Keys: `name`, `par`, `grid`, and the optional `wake` — `"long"` (the
//! shipped physics, the default) or `"short"` (the line remembers only as
//! far as it is deep, which is what lets `solver::ida_star` prove a par).

use frost::ron::{self, Kind, Val};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Rock,
    Dirt,
    Tunnel,
    Bed(crate::game::AntColor),
}

#[derive(Clone, Debug)]
pub struct Level {
    pub name: String,
    pub cols: usize,
    pub rows: usize,
    /// Row-major from the top-left corner (row 0 is the top row).
    pub tiles: Vec<Tile>,
    pub par: i32,
    pub player_start: crate::game::Cell,
    /// Wild ant spawns with their color.
    pub wild: Vec<(crate::game::Cell, crate::game::AntColor)>,
    /// The dedicated bed cell of every bed color (a fixed, tiny list).
    pub beds: Vec<(crate::game::Cell, crate::game::AntColor)>,
    /// How far the line remembers the walk — the hill's physics. Omitted
    /// by every shipped hill, which means `Wake::Long`; a hill written
    /// for the searched rules says `wake: "short"`.
    pub wake: crate::game::Wake,
}

use crate::game::{AntColor, Cell, Wake};

/// Read a `wake:` atom: `"long"` (the shipped physics, also the default)
/// or `"short"` (the searched one). Anything else refuses the hill —
/// guessing a physics for an authored puzzle is how a par starts lying.
fn wake_of(val: &Val) -> Result<Wake, String> {
    let text = match val {
        Val::Atom(text, Kind::Str) => text
            .trim_start_matches('"')
            .trim_end_matches('"')
            .to_ascii_lowercase(),
        _ => return Err("wake must be \"long\" or \"short\"".to_string()),
    };
    match text.as_str() {
        "long" => Ok(Wake::Long),
        "short" => Ok(Wake::Short),
        other => Err(format!("unknown wake '{other}'")),
    }
}

/// Build a level from grid rows, validating shape, pairing and reach of
/// beds/ants. Returns `Err(reason)` so a bad level is refused, not half-read.
/// Shorthand for the shipped physics; see `from_grid_wake`.
pub fn from_grid(name: &str, rows: &[&str], par: i32) -> Result<Level, String> {
    from_grid_wake(name, rows, par, Wake::Long)
}

fn color_of(c: char) -> Option<AntColor> {
    match c.to_ascii_lowercase() {
        'w' => Some(AntColor::White),
        'r' => Some(AntColor::Red),
        'y' => Some(AntColor::Yellow),
        'g' => Some(AntColor::Green),
        'b' => Some(AntColor::Blue),
        _ => None,
    }
}

/// Build a level under a declared wake. The grid means the same thing
/// under both: the wake is a rule about the line, not about the stone.
pub fn from_grid_wake(name: &str, rows: &[&str], par: i32, wake: Wake) -> Result<Level, String> {
    if rows.is_empty() {
        return Err(format!("{name}: no rows"));
    }
    let width = rows[0].chars().count();
    if width < 3 || rows.len() < 3 {
        return Err(format!("{name}: too small to be a hill"));
    }
    let mut tiles = Vec::with_capacity(width * rows.len());
    let mut wild = Vec::new();
    let mut beds = Vec::new();
    let mut player_start = None;
    let mut bed_colors: Vec<AntColor> = Vec::new();
    let mut wild_colors: Vec<AntColor> = Vec::new();
    for (row, line) in rows.iter().enumerate() {
        let cells: Vec<char> = line.chars().collect();
        if cells.len() != width {
            return Err(format!("{name}: row {row} is not {width} cells wide"));
        }
        for (col, ch) in cells.iter().enumerate() {
            let cell = Cell::new(col as i32, row as i32);
            match *ch {
                '#' => tiles.push(Tile::Rock),
                ',' => tiles.push(Tile::Dirt),
                '.' => tiles.push(Tile::Tunnel),
                '@' => {
                    if player_start.is_some() {
                        return Err(format!("{name}: two player starts"));
                    }
                    player_start = Some(cell);
                    tiles.push(Tile::Tunnel);
                }
                c if c.is_uppercase() => {
                    let Some(color) = color_of(c) else {
                        return Err(format!("{name}: unknown bed glyph '{c}'"));
                    };
                    if bed_colors.contains(&color) {
                        return Err(format!("{name}: two beds for one color"));
                    }
                    bed_colors.push(color);
                    beds.push((cell, color));
                    tiles.push(Tile::Bed(color));
                }
                c if c.is_lowercase() => {
                    let Some(color) = color_of(c) else {
                        return Err(format!("{name}: unknown ant glyph '{c}'"));
                    };
                    if wild_colors.contains(&color) {
                        return Err(format!("{name}: ant {c} appears twice"));
                    }
                    wild_colors.push(color);
                    wild.push((cell, color));
                    tiles.push(Tile::Tunnel);
                }
                other => return Err(format!("{name}: unknown glyph '{other}'")),
            }
        }
    }
    let player_start = player_start.ok_or_else(|| format!("{name}: no player start '@'"))?;
    if !bed_colors.contains(&AntColor::White) {
        return Err(format!("{name}: the player needs a bed 'W'"));
    }
    for color in &wild_colors {
        if !bed_colors.contains(color) {
            return Err(format!("{name}: ant {:?} has no bed", color));
        }
    }
    for color in &bed_colors {
        if *color != AntColor::White && !wild_colors.contains(color) {
            return Err(format!("{name}: bed {:?} has no ant", color));
        }
    }
    Ok(Level {
        name: name.to_string(),
        cols: width,
        rows: rows.len(),
        tiles,
        par,
        player_start,
        wild,
        beds,
        wake,
    })
}

/// Unquote a RON string atom (`Kind::Str` keeps the quotes verbatim).
fn unquote(val: &Val) -> Option<String> {
    if let Val::Atom(text, Kind::Str) = val {
        let inner = text.trim_start_matches('"').trim_end_matches('"');
        Some(inner.replace("\\\"", "\""))
    } else {
        None
    }
}

fn number(val: &Val) -> Option<i32> {
    if let Val::Atom(text, Kind::Num) = val {
        text.parse().ok()
    } else {
        None
    }
}

/// Load the `( levels: [ (name:, par:, grid: [...]), .. ] )` document.
/// Bad levels are refused loudly and skipped.
pub fn load_levels(src: &str) -> Vec<Level> {
    let mut levels = Vec::new();
    let root = match ron::parse(src) {
        Ok(root) => root,
        Err(err) => {
            log::error!("levels.ron: {err}");
            return levels;
        }
    };
    let Some(list) = field(&root, "levels") else {
        log::error!("levels.ron: no `levels` field");
        return levels;
    };
    for (i, entry) in list.kids().into_iter().enumerate() {
        let level = entry.1;
        let name = field(level, "name").and_then(unquote).unwrap_or_default();
        let name = if name.is_empty() {
            format!("level {}", i + 1)
        } else {
            name
        };
        let par = field(level, "par").and_then(number).unwrap_or(0);
        let Some(grid) = field(level, "grid") else {
            log::error!("{name}: no grid");
            continue;
        };
        let rows: Vec<String> = grid
            .kids()
            .into_iter()
            .filter_map(|(_, v)| unquote(v))
            .collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        // The wake is optional; an unreadable one refuses the hill rather
        // than guessing which physics the authored par was proved under.
        let wake = match field(level, "wake") {
            Some(val) => match wake_of(val) {
                Ok(w) => w,
                Err(reason) => {
                    log::error!("refused level {name}: {reason}");
                    continue;
                }
            },
            None => crate::game::Wake::Long,
        };
        match from_grid_wake(&name, &refs, par, wake) {
            Ok(level) => levels.push(level),
            Err(reason) => log::error!("refused level: {reason}"),
        }
    }
    if levels.is_empty() {
        log::error!("levels.ron: not one level survived");
    }
    levels
}

use frost::ron::field;
