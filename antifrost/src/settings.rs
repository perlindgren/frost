//! The player's own dials: quality, the lamp-shadow ladder, vsync,
//! the night floor, fullscreen — plain data that texts itself out and
//! parses itself back, so the same shape answers on disc and in the
//! browser's memory.

/// The lamp disk's radius when diffusion is bought: shadows fade
/// across these pixels instead of dropping in one. The ladder below
/// decides whether lamps sweep this disk with four rays or nine.
pub const LAMP_PENUMBRA: f32 = 6.0;

/// The lamp-shadow ladder: what each lit pixel pays for occlusion.
/// `None` registers no occluders at all — lamps light as if the hill
/// were glass; `One` is the single hard ray; `Four` sweeps a balanced
/// cross of the lamp's disk; `Nine` fills the 3x3 grid, the dense
/// diffusion advanced hardware buys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shadow {
    None,
    One,
    Four,
    Nine,
}

impl Shadow {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::One => "1 light",
            Self::Four => "4 light",
            Self::Nine => "9 light",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::One,
            Self::One => Self::Four,
            Self::Four => Self::Nine,
            Self::Nine => Self::None,
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "1 light" => Some(Self::One),
            "4 light" => Some(Self::Four),
            "9 light" => Some(Self::Nine),
            _ => None,
        }
    }
    /// The lamp disk, or zero for the hard single ray.
    pub fn penumbra(self) -> f32 {
        match self {
            Self::None | Self::One => 0.0,
            Self::Four | Self::Nine => LAMP_PENUMBRA,
        }
    }
    /// The rung's tap count; only ever read for a lit disk.
    pub fn taps(self) -> u32 {
        match self {
            Self::None | Self::One => 1,
            Self::Four => 4,
            Self::Nine => 9,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]

/// The stretch-pass quality tiers: the fixed buffer the hill renders
/// into, from a 720p buffer (cheap, soft) to native resolution (every
/// pixel rendered, the expensive 4K experiment included). The old
/// quarter-HD tier retired — the row's floor is now medium — and a
/// file that still names `"low"` lands on that floor rather than
/// failing. The default is High, the 1080p fixed buffer the demo
/// shipped with; Ultra lifts the fix and lets the frame render at the
pub enum Quality {
    Medium,
    High,
    VeryHigh,
    Ultra,
}

impl Quality {
    pub fn size(self) -> Option<[u32; 2]> {
        match self {
            Quality::Medium => Some([1280, 720]),
            Quality::High => Some([1920, 1080]),
            Quality::VeryHigh => Some([2560, 1440]),
            Quality::Ultra => None,
        }
    }
    /// What the row and the file call the tier: the resolution it
    /// promises, `native` for the tier that fixes none.
    pub fn label(self) -> &'static str {
        match self {
            Quality::Medium => "1280 x 720",
            Quality::High => "1920 x 1080",
            Quality::VeryHigh => "2560 x 1440",
            Quality::Ultra => "native",
        }
    }
    pub fn next(self) -> Quality {
        match self {
            Quality::Medium => Quality::High,
            Quality::High => Quality::VeryHigh,
            Quality::VeryHigh => Quality::Ultra,
            Quality::Ultra => Quality::Medium,
        }
    }
    pub fn parse(text: &str) -> Option<Quality> {
        Some(match text {
            "1280 x 720" => Quality::Medium,
            "1920 x 1080" => Quality::High,
            "2560 x 1440" => Quality::VeryHigh,
            "native" => Quality::Ultra,
            _ => return None,
        })
    }
}

/// The player's preferences, live in memory and — on native — on disc
/// beside the launch folder. Every field reaches the frame the same
/// way: vsync, filter, fullscreen, and quality flip the engine the
/// moment the row moves; ambient, light, and leaf lighting are read
/// by the draw each frame, so the settings page previews them live
/// behind the panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub vsync: bool,
    /// The stretch filter: nearest (crisp pixel blocks) or bilinear
    /// (the soft default).
    pub nearest: bool,
    pub fullscreen: bool,
    /// The render buffer's size: the row shows the resolution it
    /// names, `native` for none.
    pub quality: Quality,
    /// Ambient floor, on the [`ambient_floor`] slider: 50% is the
    /// night the game shipped with, 0% is the full dark, and 100%
    /// floors every pixel at full bright — no dimming at all.
    pub ambient: f32,
    /// Lamp strength, as a fraction of the shipped intensities —
    /// doubled past 1.0 at the slider's far end.
    pub light: f32,
    /// The halo the ants' lamps throw: the bloom chain's twelve passes, spent
    /// only while this row is on — at zero strength the engine declines to run
    /// the chain rather than computing a picture no one reads, so this is the
    /// weak hardware's rung as much as it is a look.
    ///
    /// Measured, at the buffer the hill fixes itself to: +0.39 ms a frame
    /// against a frame that is 9.71 ms there, interleaved pairs, the machine's
    /// own noise floor a quarter of that (§13 of the benchmarking notes). Never
    /// measured in a browser, which is the other place this game runs and the
    /// reason the rung exists at all.
    pub lamp_glow: bool,
    pub leaves_lit: bool,
    /// The weak-hardware answer to the light field, on a ladder:
    /// lamps stay at every rung, and only the shadows' cost steps.
    pub shadows: Shadow,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            vsync: true,
            nearest: false,
            fullscreen: true,
            quality: Quality::High,
            ambient: 0.5,
            light: 1.0,
            lamp_glow: true,
            leaves_lit: true,
            shadows: Shadow::One,
        }
    }
}

impl Settings {
    const FILE: &'static str = "antifrost.settings.ron";

    /// The file's text, in the crate's own Ron shape: flat, named,
    /// and hand-parsable by `load_levels`' little parser.
    pub fn text(&self) -> String {
        format!(
            "(\n    vsync: {},\n    filter: \"{}\",\n    fullscreen: {},\n    resolution: \"{}\",\n    ambient: {},\n    light: {},\n    lamp_glow: {},\n    leaves_lit: {},\n    shadows: \"{}\",\n)\n",
            self.vsync,
            if self.nearest { "nearest" } else { "linear" },
            self.fullscreen,
            self.quality.label(),
            self.ambient,
            self.light,
            self.lamp_glow,
            self.leaves_lit,
            self.shadows.label(),
        )
    }

    /// Parse the file's text over the defaults: an unknown name, an
    /// unreadable value, or a whole garbage file loses that one
    /// setting, not the launch.
    pub fn parse(src: &str) -> Settings {
        let d = Settings::default();
        let Ok(root) = frost::ron::parse(src) else {
            return d;
        };
        let val = |name: &str| frost::ron::field(&root, name);
        let atom = |name: &str| match val(name) {
            Some(frost::ron::Val::Atom(text, _)) => Some(text.as_str()),
            _ => None,
        };
        let boolean = |name: &str, at: bool| match atom(name) {
            Some("true") => true,
            Some("false") => false,
            _ => at,
        };
        let number = |name: &str, at: f32, lo: f32, hi: f32| {
            atom(name)
                .and_then(|t| t.parse::<f32>().ok())
                .map(|v| v.clamp(lo, hi))
                .unwrap_or(at)
        };
        let ambient = number("ambient", d.ambient, 0.0, 1.0);
        Settings {
            vsync: boolean("vsync", d.vsync),
            nearest: match atom("filter") {
                Some(t) => t.trim_matches('"') == "nearest",
                _ => d.nearest,
            },
            fullscreen: boolean("fullscreen", d.fullscreen),
            quality: atom("resolution")
                .and_then(|t| Quality::parse(t.trim_matches('"')))
                .unwrap_or(d.quality),
            ambient,
            light: number("light", d.light, 0.0, 2.0),
            lamp_glow: boolean("lamp_glow", d.lamp_glow),
            leaves_lit: boolean("leaves_lit", d.leaves_lit),
            shadows: atom("shadows")
                .and_then(|t| Shadow::parse(t.trim_matches('"')))
                .unwrap_or(d.shadows),
        }
    }

    /// The kept text, read back; missing or unreadable says default.
    /// The menu opens every launch regardless — the file speaks the
    /// settings, not the opening page. On the desktop the keeping is a
    /// file beside the launch folder; in a browser, its own memory of
    /// this site — `frost::store` answers for both worlds.
    pub fn load() -> Settings {
        match frost::store::load(Self::FILE) {
            Some(src) => Settings::parse(&src),
            None => {
                log::info!("antifrost: no {:?} yet — defaults", Self::FILE);
                Settings::default()
            }
        }
    }

    /// Keep the settings under the file's name, wherever this runs.
    /// Failure is a logged shrug, never a popped frame: a read-only
    /// folder, or a private window, costs persistence, not the game.
    pub fn save(&self) {
        if let Err(err) = frost::store::save(Self::FILE, &self.text()) {
            log::warn!("antifrost: could not save {:?}: {err}", Self::FILE);
        }
    }
}
