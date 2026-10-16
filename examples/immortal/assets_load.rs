//! Startup asset loading for the `immortal` example.
//!
//! Every sprite, sound, and font the example uses is compiled into the
//! binary in one [`Table`](frost::Table), keyed by root-relative path
//! — the same words the asset documents speak — so the example runs
//! with no asset files on disk, in any working directory, and on
//! targets without a file system. Development admits the project
//! root's loose files too, where a saved edit outranks its table
//! entry — the same code, both worlds, no fork. The older call sites
//! still speak short names, and [`key_of`] is the app's own
//! housekeeping: a name maps to the table's one path of that stem,
//! and an ambiguous name is a build error the app refuses to guess.

/// The badge's tint alpha: a faint watermark behind the play field.
const IMMORTALITY_ALPHA: f32 = 0.2;

/// The shipped world: every asset by path, every path by bytes — the
/// key and the `include_bytes!` argument are the same string, so a
/// document's reference and the build's cargo are one dialect. Loose
/// files under the admitted root shadow entries by the same path —
/// see [`frost::Assets`].
const ASSETS: frost::Table = &[
    (
        "assets/game/gameplay.ron",
        include_bytes!("../../assets/game/gameplay.ron"),
    ),
    (
        "assets/sprites/grass.png",
        include_bytes!("../../assets/sprites/grass.png"),
    ),
    (
        "assets/sprites/water_can_outline.png",
        include_bytes!("../../assets/sprites/water_can_outline.png"),
    ),
    (
        "assets/sprites/Spray1.png",
        include_bytes!("../../assets/sprites/Spray1.png"),
    ),
    (
        "assets/sprites/Spray2.png",
        include_bytes!("../../assets/sprites/Spray2.png"),
    ),
    (
        "assets/sprites/spade.png",
        include_bytes!("../../assets/sprites/spade.png"),
    ),
    (
        "assets/sprites/Tweezers.png",
        include_bytes!("../../assets/sprites/Tweezers.png"),
    ),
    (
        "assets/sprites/Waspeye1.png",
        include_bytes!("../../assets/sprites/Waspeye1.png"),
    ),
    (
        "assets/sprites/Waspeye2.png",
        include_bytes!("../../assets/sprites/Waspeye2.png"),
    ),
    (
        "assets/sprites/Bug1a.png",
        include_bytes!("../../assets/sprites/Bug1a.png"),
    ),
    (
        "assets/sprites/Bug2a.png",
        include_bytes!("../../assets/sprites/Bug2a.png"),
    ),
    (
        "assets/sprites/Bug3a.png",
        include_bytes!("../../assets/sprites/Bug3a.png"),
    ),
    (
        "assets/sprites/Worm1_crop.png",
        include_bytes!("../../assets/sprites/Worm1_crop.png"),
    ),
    (
        "assets/sprites/Worm2_crop.png",
        include_bytes!("../../assets/sprites/Worm2_crop.png"),
    ),
    (
        "assets/sprites/Lice1.png",
        include_bytes!("../../assets/sprites/Lice1.png"),
    ),
    (
        "assets/sprites/Lice2.png",
        include_bytes!("../../assets/sprites/Lice2.png"),
    ),
    (
        "assets/sprites/items.png",
        include_bytes!("../../assets/sprites/items.png"),
    ),
    (
        "assets/sprites/held_items.png",
        include_bytes!("../../assets/sprites/held_items.png"),
    ),
    (
        "assets/sprites/plant1.png",
        include_bytes!("../../assets/sprites/plant1.png"),
    ),
    (
        "assets/sprites/plant2.png",
        include_bytes!("../../assets/sprites/plant2.png"),
    ),
    (
        "assets/sprites/plant3.png",
        include_bytes!("../../assets/sprites/plant3.png"),
    ),
    (
        "assets/sprites/plant4.png",
        include_bytes!("../../assets/sprites/plant4.png"),
    ),
    (
        "assets/sprites/plant5.png",
        include_bytes!("../../assets/sprites/plant5.png"),
    ),
    (
        "assets/sprites/flower.png",
        include_bytes!("../../assets/sprites/flower.png"),
    ),
    (
        "assets/sprites/tomato.png",
        include_bytes!("../../assets/sprites/tomato.png"),
    ),
    (
        "assets/sprites/tomato_fg.png",
        include_bytes!("../../assets/sprites/tomato_fg.png"),
    ),
    (
        "assets/sprites/BasketBack.png",
        include_bytes!("../../assets/sprites/BasketBack.png"),
    ),
    (
        "assets/sprites/BasketFront.png",
        include_bytes!("../../assets/sprites/BasketFront.png"),
    ),
    (
        "assets/sprites/sustainable_immortality.png",
        include_bytes!("../../assets/sprites/sustainable_immortality.png"),
    ),
    (
        "assets/fonts/Leofont-Regular.ttf",
        include_bytes!("../../assets/fonts/Leofont-Regular.ttf"),
    ),
    (
        "assets/fonts/FiraCode-VariableFont_wght.ttf",
        include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf"),
    ),
    (
        "assets/audio/ChatterLow.wav",
        include_bytes!("../../assets/audio/ChatterLow.wav"),
    ),
    (
        "assets/audio/ChatterMid.wav",
        include_bytes!("../../assets/audio/ChatterMid.wav"),
    ),
    (
        "assets/audio/ChatterHigh.wav",
        include_bytes!("../../assets/audio/ChatterHigh.wav"),
    ),
    (
        "assets/audio/bugs_plopp1.wav",
        include_bytes!("../../assets/audio/bugs_plopp1.wav"),
    ),
    (
        "assets/audio/bugs_plopp2.wav",
        include_bytes!("../../assets/audio/bugs_plopp2.wav"),
    ),
    (
        "assets/audio/bugs_plopp3.wav",
        include_bytes!("../../assets/audio/bugs_plopp3.wav"),
    ),
    (
        "assets/audio/Ouch1.wav",
        include_bytes!("../../assets/audio/Ouch1.wav"),
    ),
    (
        "assets/audio/Ouch2.wav",
        include_bytes!("../../assets/audio/Ouch2.wav"),
    ),
    (
        "assets/audio/Ouch3.wav",
        include_bytes!("../../assets/audio/Ouch3.wav"),
    ),
    (
        "assets/audio/Ouch4.wav",
        include_bytes!("../../assets/audio/Ouch4.wav"),
    ),
    (
        "assets/audio/bugsDeath.wav",
        include_bytes!("../../assets/audio/bugsDeath.wav"),
    ),
    (
        "assets/audio/WaterFlowSoft.wav",
        include_bytes!("../../assets/audio/WaterFlowSoft.wav"),
    ),
    (
        "assets/audio/Spray.wav",
        include_bytes!("../../assets/audio/Spray.wav"),
    ),
    (
        "assets/audio/TomatoDrop.wav",
        include_bytes!("../../assets/audio/TomatoDrop.wav"),
    ),
];

/// The audio output and every clip the example plays: the device, opened
/// once at startup, and the clips it plays through — the swarm's random
/// picks, the chatter, plopp, and Ouch arrays, and the named singles: the
/// bug death, the watering loop, the spray hiss, and the tomato drop.
pub struct Sounds {
    /// The audio output, opened once at startup; every clip plays through
    /// it.
    pub device: frost::Audio,
    /// The three bug chatter clips (low, mid, high), decoded once at
    /// startup; a bug that reaches its destination while another bug is
    /// on the grass within 50 px of it chatters on one of them, the
    /// swarm's random pick.
    pub chatters: [frost::Sound; 3],
    /// The three bug plopp clips (1, 2, 3), decoded once at startup; a
    /// bug that pops up out of the grass — a batch spawn or a respawn —
    /// plops on one of them, the swarm's random pick.
    pub plops: [frost::Sound; 3],
    /// The four bug Ouch clips (1 to 4), decoded once at startup; every
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
    /// The resolver that answered this boot: the table, and the
    /// loose directories this working directory has. The `Demo`
    /// keeps it — the watched data file is re-read through it.
    pub store: frost::Assets,
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
    /// The two wasp frames: the swarm's shape swaps between them at most
    /// once per wingbeat change.
    pub wasp1: frost::Shape,
    pub wasp2: frost::Shape,
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
        let store = frost::Assets::embedded(ASSETS).with_dir(env!("CARGO_MANIFEST_DIR"));
        let mut immortality = sprite(&store, "sustainable_immortality");
        if let frost::Shape::Sprite { alpha, .. } = &mut immortality {
            *alpha = IMMORTALITY_ALPHA;
        }
        Assets {
            store: store.clone(),
            grass: sprite(&store, "grass"),
            can: sprite(&store, "water_can_outline"),
            spray1: sprite(&store, "Spray1"),
            spray2: sprite(&store, "Spray2"),
            spade: sprite(&store, "spade"),
            tweezers: sprite(&store, "Tweezers"),
            wasp1: sprite(&store, "Waspeye1"),
            wasp2: sprite(&store, "Waspeye2"),
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
                chatters: [
                    sound(&store, "ChatterLow"),
                    sound(&store, "ChatterMid"),
                    sound(&store, "ChatterHigh"),
                ],
                plops: [
                    sound(&store, "bugs_plopp1"),
                    sound(&store, "bugs_plopp2"),
                    sound(&store, "bugs_plopp3"),
                ],
                ajs: [
                    sound(&store, "Ouch1"),
                    sound(&store, "Ouch2"),
                    sound(&store, "Ouch3"),
                    sound(&store, "Ouch4"),
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
/// missing picture. The name is the app's own shorthand; the store
/// hears only the path [`key_of`] finds for it.
fn ask(store: &frost::Assets, name: &str, exts: &[&str]) -> std::borrow::Cow<'static, [u8]> {
    store
        .asset(key_of(name), exts)
        .unwrap_or_else(|| panic!("the asset table does not carry '{name}'"))
}

/// The table's one path whose file stem is this name — the app's
/// housekeeping over its own keys, not the resolver's guessing:
/// a stem no entry wears, or two entries wearing one stem, are both
/// build errors this example refuses at the first load rather than
/// choices it would have to live with.
fn key_of(name: &str) -> &'static str {
    let found: Vec<&'static str> = ASSETS
        .iter()
        .map(|(path, _)| *path)
        .filter(|path| {
            std::path::Path::new(path)
                .file_stem()
                .is_some_and(|stem| stem == name)
        })
        .collect();
    match found.as_slice() {
        [one] => one,
        [] => panic!("the asset table wears no '{name}'"),
        many => panic!("the asset table wears {name} twice: {many:?} — a stem is one asset"),
    }
}
