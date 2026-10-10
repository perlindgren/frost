//! Startup asset loading for the `immortal` example.
//!
//! Every sprite, sound, and font the example uses is compiled into the
//! binary in one [`Table`](frost::Table), named by file stem, so the
//! example runs with no asset files on disk — in any working directory,
//! and on targets without a file system. Development admits the loose
//! `assets/` directories too, where a saved edit outranks its table
//! entry — the same code, both worlds, no fork. [`Assets::load`] decodes
//! it all in one place; `main` hands it to the plant node, the scene,
//! and the `Demo`.

/// The badge's tint alpha: a faint watermark behind the play field.
const IMMORTALITY_ALPHA: f32 = 0.2;

/// The shipped world: every asset by stem, every stem by bytes.
/// The loose `assets/` directories shadow entries by the same name —
/// see [`frost::Assets`].
const ASSETS: frost::Table = &[
    ("grass", include_bytes!("../../assets/sprites/grass.png")),
    (
        "water_can_outline",
        include_bytes!("../../assets/sprites/water_can_outline.png"),
    ),
    ("Spray1", include_bytes!("../../assets/sprites/Spray1.png")),
    ("Spray2", include_bytes!("../../assets/sprites/Spray2.png")),
    ("spade", include_bytes!("../../assets/sprites/spade.png")),
    (
        "Tweezers",
        include_bytes!("../../assets/sprites/Tweezers.png"),
    ),
    (
        "Getingeye1",
        include_bytes!("../../assets/sprites/Getingeye1.png"),
    ),
    (
        "Getingeye2",
        include_bytes!("../../assets/sprites/Getingeye2.png"),
    ),
    ("Bug1a", include_bytes!("../../assets/sprites/Bug1a.png")),
    ("Bug2a", include_bytes!("../../assets/sprites/Bug2a.png")),
    ("Bug3a", include_bytes!("../../assets/sprites/Bug3a.png")),
    (
        "Worm1_crop",
        include_bytes!("../../assets/sprites/Worm1_crop.png"),
    ),
    (
        "Worm2_crop",
        include_bytes!("../../assets/sprites/Worm2_crop.png"),
    ),
    ("Lice1", include_bytes!("../../assets/sprites/Lice1.png")),
    ("Lice2", include_bytes!("../../assets/sprites/Lice2.png")),
    ("items", include_bytes!("../../assets/sprites/items.png")),
    (
        "held_items",
        include_bytes!("../../assets/sprites/held_items.png"),
    ),
    ("plant1", include_bytes!("../../assets/sprites/plant1.png")),
    ("plant2", include_bytes!("../../assets/sprites/plant2.png")),
    ("plant3", include_bytes!("../../assets/sprites/plant3.png")),
    ("plant4", include_bytes!("../../assets/sprites/plant4.png")),
    ("plant5", include_bytes!("../../assets/sprites/plant5.png")),
    ("flower", include_bytes!("../../assets/sprites/flower.png")),
    ("tomato", include_bytes!("../../assets/sprites/tomato.png")),
    (
        "tomato_fg",
        include_bytes!("../../assets/sprites/tomato_fg.png"),
    ),
    (
        "BasketBack",
        include_bytes!("../../assets/sprites/BasketBack.png"),
    ),
    (
        "BasketFront",
        include_bytes!("../../assets/sprites/BasketFront.png"),
    ),
    (
        "sustainable_immortality",
        include_bytes!("../../assets/sprites/sustainable_immortality.png"),
    ),
    (
        "Leofont-Regular",
        include_bytes!("../../assets/fonts/Leofont-Regular.ttf"),
    ),
    (
        "FiraCode-VariableFont_wght",
        include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf"),
    ),
    (
        "TjatterLow",
        include_bytes!("../../assets/audio/TjatterLow.wav"),
    ),
    (
        "TjatterMid",
        include_bytes!("../../assets/audio/TjatterMid.wav"),
    ),
    (
        "TjatterHigh",
        include_bytes!("../../assets/audio/TjatterHigh.wav"),
    ),
    (
        "bugs_plopp1",
        include_bytes!("../../assets/audio/bugs_plopp1.wav"),
    ),
    (
        "bugs_plopp2",
        include_bytes!("../../assets/audio/bugs_plopp2.wav"),
    ),
    (
        "bugs_plopp3",
        include_bytes!("../../assets/audio/bugs_plopp3.wav"),
    ),
    ("Aj1", include_bytes!("../../assets/audio/Aj1.wav")),
    ("Aj2", include_bytes!("../../assets/audio/Aj2.wav")),
    ("Aj3", include_bytes!("../../assets/audio/Aj3.wav")),
    ("Aj4", include_bytes!("../../assets/audio/Aj4.wav")),
    (
        "bugsDeath",
        include_bytes!("../../assets/audio/bugsDeath.wav"),
    ),
    (
        "WaterFlowSoft",
        include_bytes!("../../assets/audio/WaterFlowSoft.wav"),
    ),
    ("Spray", include_bytes!("../../assets/audio/Spray.wav")),
    (
        "TomatoDrop",
        include_bytes!("../../assets/audio/TomatoDrop.wav"),
    ),
];

/// The audio output and every clip the example plays: the device, opened
/// once at startup, and the clips it plays through — the swarm's random
/// picks, the tjatter, plopp, and Aj arrays, and the named singles: the
/// bug death, the watering loop, the spray hiss, and the tomato drop.
pub struct Sounds {
    /// The audio output, opened once at startup; every clip plays through
    /// it.
    pub device: frost::Audio,
    /// The three bug tjatter clips (low, mid, high), decoded once at
    /// startup; a bug that reaches its destination while another bug is
    /// on the grass within 50 px of it chatters on one of them, the
    /// swarm's random pick.
    pub tjatters: [frost::Sound; 3],
    /// The three bug plopp clips (1, 2, 3), decoded once at startup; a
    /// bug that pops up out of the grass — a batch spawn or a respawn —
    /// plops on one of them, the swarm's random pick.
    pub plops: [frost::Sound; 3],
    /// The four bug Aj clips (1 to 4), decoded once at startup; every
    /// time the mist drops a bug's health the bug cries out on one of
    /// them, the swarm's random pick.
    pub ajs: [frost::Sound; 4],
    /// The bug death clip, `assets/audio/bugsDeath.wav`, decoded once at
    /// startup; it plays, at the default volume, for every bug a mist hit
    /// kills.
    pub death: frost::Sound,
    /// The watering sound, `assets/audio/WaterFlowSoft.wav`, decoded once
    /// at startup; the audio output's one loop, playing while water
    /// actually pours out of the spout and silenced the frame pouring
    /// stops.
    pub pour: frost::Sound,
    /// The spray hiss, `assets/audio/Spray.wav`, decoded once at startup;
    /// it plays, at 0.5 volume, on every fresh press that triggers a
    /// spray burst.
    pub spray: frost::Sound,
    /// The tomato drop clip, `assets/audio/TomatoDrop.wav`, decoded once
    /// at startup; it plays, at the default volume, on the frame a
    /// fallen, overgrown tomato reaches its destination.
    pub tomato_drop: frost::Sound,
}

/// Everything the example loads at startup: the sprite shapes, the
/// game-over overlay's font, the diagnostics overlay's font, and the audio
/// — the output and every clip — embedded at compile time with
/// `include_bytes!` and decoded by [`Assets::load`].
pub struct Assets {
    /// The grass photo, exactly the window size: stretched to fill the
    /// window by the process.
    pub grass: frost::Shape,
    /// The water can's outline, the active-tool sprite for
    /// `Tool::WaterCan`.
    pub can: frost::Shape,
    /// The spray can's at-rest frame, the active-tool sprite for
    /// `Tool::SprayCan`.
    pub spray1: frost::Shape,
    /// The spray can's pressed frame, swapped into the tool node while the
    /// button is down.
    pub spray2: frost::Shape,
    /// The garden spade, the active-tool sprite for `Tool::Spade`.
    pub spade: frost::Shape,
    /// The tweezers, the active-tool sprite for `Tool::Tweezers`.
    pub tweezers: frost::Shape,
    /// The two viper frames: the swarm's shape swaps between them at most
    /// once per wingbeat change.
    pub viper1: frost::Shape,
    pub viper2: frost::Shape,
    /// The three bug walk frames: a slot's shape swaps when its bug's walk
    /// frame changes.
    pub bug1: frost::Shape,
    pub bug2: frost::Shape,
    pub bug3: frost::Shape,
    /// The two worm peristaltic frames: a slot's shape swaps between them
    /// as the worm's body beats.
    pub worm1: frost::Shape,
    pub worm2: frost::Shape,
    /// The two louse walk frames — art that walks to the left; the louse
    /// swarm's slot shapes swap between them as cheap `Arc` clones.
    pub lice1: frost::Shape,
    pub lice2: frost::Shape,
    /// The inventory panel, mid left: the shelf, painted with four bays,
    /// the tweezers resting in the top one, then the spade, the spray can,
    /// and the watering can at the bottom.
    pub items: frost::Shape,
    /// The held-items panel in the bottom right: the active tool in the
    /// left half, the stored one in the right.
    pub held_items: frost::Shape,
    /// The five plant slices in chain order, from the root joint up.
    pub plant1: frost::Shape,
    pub plant2: frost::Shape,
    pub plant3: frost::Shape,
    pub plant4: frost::Shape,
    pub plant5: frost::Shape,
    /// The bloom's flower leaf, under its tomato.
    pub flower: frost::Shape,
    /// The tomato's white body leaf, the bloom's background.
    pub tomato: frost::Shape,
    /// The tomato's calyx-and-stem leaf, the bloom's foreground.
    pub tomato_fg: frost::Shape,
    /// The basket's back half, under the fruit.
    pub basket_back: frost::Shape,
    /// The basket's front half, over the fruit.
    pub basket_front: frost::Shape,
    /// The immortality badge, tinted down to a faint watermark.
    pub immortality: frost::Shape,
    /// The game-over overlay's font bytes, embedded at compile time: the
    /// "Game Over" title and the Play button's label are built from them.
    pub font: std::borrow::Cow<'static, [u8]>,
    /// The diagnostics overlay's font bytes, embedded at compile time: the
    /// readout lines and the strip charts are built from them.
    pub diag_font: std::borrow::Cow<'static, [u8]>,
    /// The audio output and every clip the example plays, decoded once at
    /// startup.
    pub sounds: Sounds,
}

impl Assets {
    /// Decodes every embedded sprite and sound, opening the audio output
    /// first. The immortality badge is tinted down to a faint watermark on
    /// the way in.
    pub fn load() -> Self {
        let audio = frost::Audio::new().expect("failed to open the audio output device");
        // The resolver of this run: the table beneath, the loose
        // directories above it when this working directory has them.
        let store = frost::Assets::embedded(ASSETS)
            .with_dir("assets/sprites")
            .with_dir("assets/audio")
            .with_dir("assets/fonts");
        let mut immortality = sprite(&store, "sustainable_immortality");
        if let frost::Shape::Sprite { alpha, .. } = &mut immortality {
            *alpha = IMMORTALITY_ALPHA;
        }
        Assets {
            grass: sprite(&store, "grass"),
            can: sprite(&store, "water_can_outline"),
            spray1: sprite(&store, "Spray1"),
            spray2: sprite(&store, "Spray2"),
            spade: sprite(&store, "spade"),
            tweezers: sprite(&store, "Tweezers"),
            viper1: sprite(&store, "Getingeye1"),
            viper2: sprite(&store, "Getingeye2"),
            bug1: sprite(&store, "Bug1a"),
            bug2: sprite(&store, "Bug2a"),
            bug3: sprite(&store, "Bug3a"),
            worm1: sprite(&store, "Worm1_crop"),
            worm2: sprite(&store, "Worm2_crop"),
            lice1: sprite(&store, "Lice1"),
            lice2: sprite(&store, "Lice2"),
            items: sprite(&store, "items"),
            held_items: sprite(&store, "held_items"),
            plant1: sprite(&store, "plant1"),
            plant2: sprite(&store, "plant2"),
            plant3: sprite(&store, "plant3"),
            plant4: sprite(&store, "plant4"),
            plant5: sprite(&store, "plant5"),
            flower: sprite(&store, "flower"),
            tomato: sprite(&store, "tomato"),
            tomato_fg: sprite(&store, "tomato_fg"),
            basket_back: sprite(&store, "BasketBack"),
            basket_front: sprite(&store, "BasketFront"),
            immortality,
            font: ask(&store, "Leofont-Regular", &["ttf"]),
            diag_font: ask(&store, "FiraCode-VariableFont_wght", &["ttf"]),
            sounds: Sounds {
                device: audio,
                tjatters: [
                    sound(&store, "TjatterLow"),
                    sound(&store, "TjatterMid"),
                    sound(&store, "TjatterHigh"),
                ],
                plops: [
                    sound(&store, "bugs_plopp1"),
                    sound(&store, "bugs_plopp2"),
                    sound(&store, "bugs_plopp3"),
                ],
                ajs: [
                    sound(&store, "Aj1"),
                    sound(&store, "Aj2"),
                    sound(&store, "Aj3"),
                    sound(&store, "Aj4"),
                ],
                death: sound(&store, "bugsDeath"),
                pour: sound(&store, "WaterFlowSoft"),
                spray: sound(&store, "Spray"),
                tomato_drop: sound(&store, "TomatoDrop"),
            },
        }
    }
}

/// Decodes the embedded sprite `assets/sprites/{name}`, panicking with
/// the asset's path on failure.
/// Asks the store for a sprite by name and decodes it: the name is
/// the file stem, in the table and in the loose directories alike.
fn sprite(store: &frost::Assets, name: &str) -> frost::Shape {
    frost::Shape::sprite_bytes(ask(store, name, &["png"]).as_ref())
        .unwrap_or_else(|err| panic!("failed to decode the sprite '{name}': {err}"))
}

/// Asks the store for a sound by name and decodes it.
fn sound(store: &frost::Assets, name: &str) -> frost::Sound {
    frost::Sound::load_bytes(ask(store, name, &["wav"]).as_ref())
        .unwrap_or_else(|err| panic!("failed to decode the sound '{name}': {err}"))
}

/// The bytes a name means in one of its formats, with the house
/// refusal: an asset the table forgot is a build error, not a
/// missing picture.
fn ask(store: &frost::Assets, name: &str, exts: &[&str]) -> std::borrow::Cow<'static, [u8]> {
    store
        .asset(name, exts)
        .unwrap_or_else(|| panic!("the asset table does not carry '{name}'"))
}
