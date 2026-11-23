//! Standalone frost rack GUI and safe main-thread audio controller.
#![deny(unsafe_code)]

pub use nice_rack_engine as routing;
use serde::{Deserialize, Serialize};

/// Local device configuration, rack instances, native states and editor windows.
pub type Settings = nice_common::Settings<RackPreferences>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RackPreferences {
    pub plugins: Vec<SavedPlugin>,
    /// The level, in dBFS, a wire's meter has to reach before its stub — the
    /// small box on the wire, what the meter looks like when nobody has it
    /// open — leaves the colour of the wire it sits on and takes the green a
    /// level is drawn in. Below the gate the stub only says "a meter is
    /// watching this wire"; at it, the stub says "and something is coming
    /// through loud enough to matter". A clip outranks both, so a stub that
    /// went red stays red whatever the gate says.
    ///
    /// The field carries no `#[serde(default)]` of its own because the
    /// attribute on this struct does reach in here, unlike on [`SavedPlugin`]
    /// where each field had to ask for itself: a missing gate comes from
    /// [`Default`], and the hand-written impl below is what answers for it. So
    /// a rack saved before the gate existed opens at
    /// [`METER_GATE_DEFAULT_DB`] — the colour that rack was already showing,
    /// since every stub used to rest.
    pub meter_gate_db: f32,
    /// The trim on the master output wire in dB — the gain the OUT jack applies
    /// to whatever the mixers have summed before the signal leaves the rack.
    /// A rack with parallel branches sums at unity, so this is the trim that
    /// keeps a loud mix off the DAC. Stored the way a card's wire trims are,
    /// and a file written before the output stage existed names none, which
    /// is 0 dB — the gain such a rack was already running.
    pub output_gain_db: f32,
    /// The trim on the input jack wire in dB — the gain the IN card applies
    /// to the captured signal before any card in the rack sees it. This is
    /// the stage that sets what the rack is fed, so the input meter reads the
    /// signal after it. Stored the way the output trim is, and a file written
    /// before the input stage existed names none, which is 0 dB — the gain
    /// such a rack was already running.
    pub input_gain_db: f32,
}

/// Written by hand rather than derived, so the one preference that does not
/// start at zero has one place that says what it starts at. `Default` is also
/// what `#[serde(default)]` above falls back to for a field an older file
/// never named, which is why the gate's default lives here and not on a
/// per-field attribute.
impl Default for RackPreferences {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            meter_gate_db: METER_GATE_DEFAULT_DB,
            output_gain_db: 0.0,
            input_gain_db: 0.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPlugin {
    /// Structural routing stored once on the first entry; absent imports legacy lanes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<layout::Flow>,
    pub id: u64,
    pub stage: u64,
    /// Which branch of its stage this card chains onto, keyed by the branch's
    /// first card; `None` is a branch of one. Like `stage`, it is a grouping
    /// label, not an audio fact. `#[serde(default)]` on its own, so a file
    /// written before chains existed still loads and every card lands in a
    /// branch of its own — the wiring that file's racks have always had.
    #[serde(default)]
    pub branch: Option<u64>,
    /// The columns of its stage this card's lane runs across, as a
    /// half-open `(start, end)` pair — the same value on every card of the
    /// lane. A lane may start after the stage's split and end before its
    /// merge, which is how a local fork around one column is remembered;
    /// `None` is the whole stage, the shape every lane had before local
    /// forks existed. `#[serde(default)]` on its own, so a file written
    /// before spans existed still loads and lays out exactly as it did.
    #[serde(default)]
    pub span: Option<(u64, u64)>,
    pub path: std::path::PathBuf,
    pub class: String,
    pub name: String,
    pub state: Option<Vec<u8>>,
    pub editor: Option<EditorGeometry>,
    /// Gain trim, in dB, the rack applies on this instance's input wire and on
    /// its output wire. Gain staging is part of how a rack was left, so it is
    /// saved with the rack and resumed on start. Both fields are
    /// `#[serde(default)]` on their own — the attribute on
    /// [`RackPreferences`] does not reach in here — so a file written before
    /// trims existed still loads, and a trim the file does not name reads
    /// unity, which is the gain that rack was running.
    ///
    /// What the meters *read* is deliberately absent. A meter measures the
    /// signal in front of it and holds a clip until somebody clicks it away;
    /// a clip is a report about audio that has already gone past, so carrying
    /// one into a resumed rack would paint the master and half the wires red
    /// over silence, and would destroy the one thing the latched red is for —
    /// the upstream-most red names the first stage that clipped. So the
    /// record holds the trims and not the meters, a resumed rack's meters
    /// start green (`PeakMeter::latching` is born unlatched, and nothing here
    /// asks it to remember more), and every red is earned again from the
    /// first block.
    #[serde(default)]
    pub gain_in: f32,
    #[serde(default)]
    pub gain_out: f32,
    /// Whether the operator left this instance's input wire or output wire
    /// broken — an open circuit, which passes nothing.
    ///
    /// A break belongs in the file for the reason the trims do and the meters
    /// do not: it is a switch the operator left thrown, a decision about how the
    /// rack is wired, and a resumed rack that closed it would be rewiring the
    /// rack behind their back. Both fields are `#[serde(default)]` on their own,
    /// so a file written before wires could break still loads, and a wire the
    /// file does not name reads closed — the state every cable has always
    /// started in.
    #[serde(default)]
    pub open_in: bool,
    #[serde(default)]
    pub open_out: bool,
    /// Unity dry route around this card, with its trims temporarily disabled.
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default)]
    pub paused: bool,
}
/// Half the range a wire's gain trim spans, in dB: the track is drawn across
/// ±24 dB, the drag clamps to it, a typed number is answered at it, the
/// restored value is clamped back to it, and the engine's own ceiling of ×16
/// linear sits just outside, so a trim can never be shown at a gain the audio
/// path would not apply.
pub const TRIM_DB: f32 = 24.0;
/// The gate a rack that was never told otherwise starts with, in dBFS, and the
/// row the settings page calls its default. Quiet enough that a wire carrying
/// anything worth hearing lights its stub, loud enough that the noise floor of
/// a wire nobody is playing — and the silence a meter drains to when a chain
/// stops — stays in the wire's own colour.
pub const METER_GATE_DEFAULT_DB: f32 = -60.0;
/// The gates the settings page offers, quietest first. Every one sits above the
/// meters' silence floor ([`nice_common::meter::SILENCE_DB`], -120 dBFS), so a
/// gate is always a level a meter can be either side of rather than a word for
/// "always"; the loudest row asks for a hot signal, which in practice is how an
/// operator keeps the stubs resting without switching the reading off.
pub const METER_GATES: [f32; 10] = [
    -90.0, -72.0, -60.0, -48.0, -40.0, -32.0, -24.0, -16.0, -12.0, -6.0,
];
/// How a gate is written in the settings file and in the settings page's
/// dropdown: a whole-number decibel string, the same words in both places, so
/// the value a dropdown row carries is also a number a hand-edited file can
/// hold. The unit is not in it — the row that offers these says "(dB)".
pub fn meter_gate_text(db: f32) -> String {
    format!("{db:.0}")
}
/// Physical screen position and logical editor content size. Optional position
/// accommodates platforms which do not expose top-level placement (Wayland).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorGeometry {
    pub position: Option<[i32; 2]>,
    pub size: [u32; 2],
}
impl nice_common::Preferences for RackPreferences {
    const FILE_NAME: &'static str = "nice-rack-settings.ron";
    const APP_NAME: &'static str = "nice-rack";
    fn is_blank(&self) -> bool {
        self.plugins.is_empty()
    }
}

mod editors;
mod gui;
mod history;
mod persistence;
pub mod runtime;
pub use gui::run;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub mod layout;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_states_and_editor_geometry_round_trip_independently() {
        let path = frost::scratch::claim("rack-settings").join("rack.ron");
        let mut settings = Settings::default();
        for (id, state, x, open_out) in
            [(1, vec![0, 255, 1], 20, true), (3, vec![42, 8], 840, false)]
        {
            settings.preferences.plugins.push(SavedPlugin {
                routing: None,
                id,
                stage: 1,
                // One card carries a chain key and one does not, so the file
                // is read for both shapes and the key survives as written.
                branch: (id == 1).then_some(1),
                span: None,
                path: "/plugins/tuner.clap".into(),
                class: "same-class".into(),
                name: "Tuner".into(),
                state: Some(state),
                editor: Some(EditorGeometry {
                    position: Some([x, 40]),
                    size: [640, 400],
                }),
                gain_in: -6.0,
                gain_out: 2.5,
                // One card left with a cable broken and one without, so the
                // file is read for both answers and not just the interesting
                // one: a saved rack that says nothing is broken has to say so
                // in as many words as one that says something is.
                open_in: false,
                open_out,
                bypassed: id == 1,
                paused: id == 3,
            });
        }
        settings.preferences.plugins[0].routing = Some(layout::Flow::parallel(vec![
            layout::Flow::Plugin(1),
            layout::Flow::Plugin(3),
        ]));
        settings.save_to(&path).unwrap();
        let (loaded, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(loaded, settings);
        Settings::save_window_to(
            &path,
            nice_common::WindowGeom {
                x: None,
                y: None,
                width: 1000.0,
                height: 600.0,
            },
        )
        .unwrap();
        assert_eq!(
            Settings::load_from(&path).0.preferences,
            settings.preferences
        );
    }

    #[test]
    fn a_settings_file_from_before_the_meter_gate_opens_at_sixty_below() {
        // Same trick as the file above: the interesting part is the line that
        // is not there. A rack saved before stubs had a gate has to open
        // showing what it showed then — every stub resting, which is what a
        // gate nobody chose means — and not come back as a file the rack
        // refuses to read.
        let path = frost::scratch::claim("rack-settings-gate-legacy").join("rack.ron");
        std::fs::write(
            &path,
            r#"(
    plugins: [
        (
            id: 2,
            stage: 1,
            path: "/plugins/tuner.clap",
            class: "same-class",
            name: "Tuner",
            state: None,
            editor: None,
            gain_in: -3.0,
            gain_out: 0.0,
        ),
    ],
)"#,
        )
        .unwrap();
        let (loaded, warning) = Settings::load_from(&path);
        assert_eq!(warning, None, "a legacy file is not a damaged one");
        assert_eq!(loaded.preferences.meter_gate_db, METER_GATE_DEFAULT_DB);
        // The same argument a feature later, for the OUT jack's trim: a file
        // that predates the output stage says nothing, and nothing is 0 dB —
        // the gain that rack was already running.
        assert_eq!(loaded.preferences.output_gain_db, 0.0);
        // And the gate joins the file on the next save rather than replacing
        // the trims the file did write.
        assert_eq!(loaded.preferences.plugins[0].gain_in, -3.0);
        assert_eq!(loaded.preferences.plugins[0].gain_out, 0.0);
        // The same argument one feature later, for the fields this file also
        // predates: a rack saved before a wire could be broken is a rack with
        // nothing broken, which is the answer `#[serde(default)]` gives, and
        // not a file the rack refuses to open.
        assert!(
            !loaded.preferences.plugins[0].open_in && !loaded.preferences.plugins[0].open_out,
            "a file that predates breaks opens with every cable closed"
        );
    }

    #[test]
    fn every_offered_meter_gate_survives_a_trip_through_the_settings_file() {
        // The page hands the rack a number as text, the rack keeps a number and
        // writes the file, and the next start reads the file back. All three
        // have to name the same gate, or a chosen threshold is gone by the
        // second start and the stubs quietly go back to resting.
        let path = frost::scratch::claim("rack-settings-gate").join("rack.ron");
        let mut settings = Settings::default();
        assert_eq!(
            settings.preferences.meter_gate_db, METER_GATE_DEFAULT_DB,
            "and the default the page offers is the default a new rack holds"
        );
        assert!(
            METER_GATES.contains(&METER_GATE_DEFAULT_DB),
            "otherwise the row the page calls the default is one nobody can pick back"
        );
        for gate in METER_GATES {
            settings.preferences.meter_gate_db = gate;
            settings.save_to(&path).unwrap();
            let (loaded, warning) = Settings::load_from(&path);
            assert_eq!(warning, None);
            assert_eq!(loaded, settings, "the whole file, not just the gate");
            assert_eq!(
                meter_gate_text(gate).parse::<f32>(),
                Ok(gate),
                "the dropdown's own words for {gate} come back as {gate}"
            );
        }
    }

    #[test]
    fn a_settings_file_from_before_trims_still_loads_at_unity() {
        // The point of this file is what it does not say. A rack saved before
        // wires had trims carries no gain for them, and the whole reason the
        // two fields default per field instead of the record requiring them is
        // that such a rack still has to open — at unity, the gain it was
        // actually running, not at whatever `Default` happened to invent.
        let path = frost::scratch::claim("rack-settings-legacy").join("rack.ron");
        std::fs::write(
            &path,
            r#"(
    plugins: [
        (
            id: 2,
            stage: 1,
            path: "/plugins/tuner.clap",
            class: "same-class",
            name: "Tuner",
            state: Some([0, 255, 1]),
            editor: Some((
                position: Some([20, 40]),
                size: (640, 400),
            )),
        ),
    ],
)"#,
        )
        .unwrap();
        let (loaded, warning) = Settings::load_from(&path);
        assert_eq!(warning, None, "a legacy file is not a damaged one");
        assert_eq!(loaded.preferences.plugins[0].gain_in, 0.0);
        assert_eq!(loaded.preferences.plugins[0].gain_out, 0.0);
        // And what it loads is what gets written back: the trim joins the
        // record on the next save rather than replacing the file's own words.
        assert_eq!(loaded.preferences.plugins[0].name, "Tuner");
        assert_eq!(loaded.preferences.plugins[0].state, Some(vec![0, 255, 1]));
    }
}
