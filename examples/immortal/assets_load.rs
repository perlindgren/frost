//! Startup asset loading for the `immortal` example.
//!
//! Every sprite and sound the example uses is embedded into the binary
//! with `include_bytes!`, pinned to the crate's `assets/` directory at
//! compile time, so the example runs with no asset files on disk — in any
//! working directory, and on targets without a file system.
//! [`Assets::load`] decodes it all in one place; `main` hands it to the
//! plant node, the scene, and the `Demo`.

/// The badge's tint alpha: a faint watermark behind the play field.
const IMMORTALITY_ALPHA: f32 = 0.2;

/// The grass photo, `assets/sprites/grass.png`.
const GRASS: &[u8] = include_bytes!("../../assets/sprites/grass.png");

/// The water can's outline, `assets/sprites/water_can_outline.png`.
const WATER_CAN_OUTLINE: &[u8] = include_bytes!("../../assets/sprites/water_can_outline.png");

/// The spray can's at-rest frame, `assets/sprites/Spray1.png`.
const SPRAY1: &[u8] = include_bytes!("../../assets/sprites/Spray1.png");

/// The spray can's pressed frame, `assets/sprites/Spray2.png`.
const SPRAY2: &[u8] = include_bytes!("../../assets/sprites/Spray2.png");

/// The first viper frame, `assets/sprites/Getingeye1.png`.
const GETINGEYE1: &[u8] = include_bytes!("../../assets/sprites/Getingeye1.png");

/// The second viper frame, `assets/sprites/Getingeye2.png`.
const GETINGEYE2: &[u8] = include_bytes!("../../assets/sprites/Getingeye2.png");

/// The first bug walk frame, `assets/sprites/Bug1a.png`.
const BUG1A: &[u8] = include_bytes!("../../assets/sprites/Bug1a.png");

/// The second bug walk frame, `assets/sprites/Bug2a.png`.
const BUG2A: &[u8] = include_bytes!("../../assets/sprites/Bug2a.png");

/// The third bug walk frame, `assets/sprites/Bug3a.png`.
const BUG3A: &[u8] = include_bytes!("../../assets/sprites/Bug3a.png");

/// The inventory panel, `assets/sprites/items.png`.
const ITEMS: &[u8] = include_bytes!("../../assets/sprites/items.png");

/// The held-items panel, `assets/sprites/held_items.png`.
const HELD_ITEMS: &[u8] = include_bytes!("../../assets/sprites/held_items.png");

/// The first plant slice, `assets/sprites/plant1.png`.
const PLANT1: &[u8] = include_bytes!("../../assets/sprites/plant1.png");

/// The second plant slice, `assets/sprites/plant2.png`.
const PLANT2: &[u8] = include_bytes!("../../assets/sprites/plant2.png");

/// The third plant slice, `assets/sprites/plant3.png`.
const PLANT3: &[u8] = include_bytes!("../../assets/sprites/plant3.png");

/// The fourth plant slice, `assets/sprites/plant4.png`.
const PLANT4: &[u8] = include_bytes!("../../assets/sprites/plant4.png");

/// The fifth plant slice, `assets/sprites/plant5.png`.
const PLANT5: &[u8] = include_bytes!("../../assets/sprites/plant5.png");

/// The bloom's flower leaf, `assets/sprites/flower.png`.
const FLOWER: &[u8] = include_bytes!("../../assets/sprites/flower.png");

/// The tomato's white body leaf, `assets/sprites/tomato.png`.
const TOMATO: &[u8] = include_bytes!("../../assets/sprites/tomato.png");

/// The tomato's calyx-and-stem leaf, `assets/sprites/tomato_fg.png`.
const TOMATO_FG: &[u8] = include_bytes!("../../assets/sprites/tomato_fg.png");

/// The basket's back half, `assets/sprites/BasketBack.png`.
const BASKET_BACK: &[u8] = include_bytes!("../../assets/sprites/BasketBack.png");

/// The basket's front half, `assets/sprites/BasketFront.png`.
const BASKET_FRONT: &[u8] = include_bytes!("../../assets/sprites/BasketFront.png");

/// The immortality badge, `assets/sprites/sustainable_immortality.png`.
const SUSTAINABLE_IMMORTALITY: &[u8] =
    include_bytes!("../../assets/sprites/sustainable_immortality.png");

/// The low tjatter clip, `assets/audio/TjatterLow.wav`.
const TJATTER_LOW: &[u8] = include_bytes!("../../assets/audio/TjatterLow.wav");

/// The mid tjatter clip, `assets/audio/TjatterMid.wav`.
const TJATTER_MID: &[u8] = include_bytes!("../../assets/audio/TjatterMid.wav");

/// The high tjatter clip, `assets/audio/TjatterHigh.wav`.
const TJATTER_HIGH: &[u8] = include_bytes!("../../assets/audio/TjatterHigh.wav");

/// The first plopp clip, `assets/audio/bugs_plopp1.wav`.
const BUGS_PLOPP1: &[u8] = include_bytes!("../../assets/audio/bugs_plopp1.wav");

/// The second plopp clip, `assets/audio/bugs_plopp2.wav`.
const BUGS_PLOPP2: &[u8] = include_bytes!("../../assets/audio/bugs_plopp2.wav");

/// The third plopp clip, `assets/audio/bugs_plopp3.wav`.
const BUGS_PLOPP3: &[u8] = include_bytes!("../../assets/audio/bugs_plopp3.wav");

/// The first Aj clip, `assets/audio/Aj1.wav`.
const AJ1: &[u8] = include_bytes!("../../assets/audio/Aj1.wav");

/// The second Aj clip, `assets/audio/Aj2.wav`.
const AJ2: &[u8] = include_bytes!("../../assets/audio/Aj2.wav");

/// The third Aj clip, `assets/audio/Aj3.wav`.
const AJ3: &[u8] = include_bytes!("../../assets/audio/Aj3.wav");

/// The fourth Aj clip, `assets/audio/Aj4.wav`.
const AJ4: &[u8] = include_bytes!("../../assets/audio/Aj4.wav");

/// The bug death clip, `assets/audio/bugsDeath.wav`.
const BUGS_DEATH: &[u8] = include_bytes!("../../assets/audio/bugsDeath.wav");

/// The watering loop, `assets/audio/WaterFlowSoft.wav`.
const WATER_FLOW_SOFT: &[u8] = include_bytes!("../../assets/audio/WaterFlowSoft.wav");

/// The spray hiss, `assets/audio/Spray.wav`.
const SPRAY: &[u8] = include_bytes!("../../assets/audio/Spray.wav");

/// The tomato drop clip, `assets/audio/tomatoDrop.wav`.
const TOMATO_DROP: &[u8] = include_bytes!("../../assets/audio/TomatoDrop.wav");

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

/// Everything the example loads at startup: the sprite shapes and the
/// audio — the output and every clip — embedded at compile time with
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
    /// The two viper frames: the swarm's shape swaps between them at most
    /// once per wingbeat change.
    pub viper1: frost::Shape,
    pub viper2: frost::Shape,
    /// The three bug walk frames: a slot's shape swaps when its bug's walk
    /// frame changes.
    pub bug1: frost::Shape,
    pub bug2: frost::Shape,
    pub bug3: frost::Shape,
    /// The inventory panel, mid left: four slots, 0 to 3 from the top,
    /// the spray can resting in slot 2 and the watering can in slot 3.
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
        let mut immortality = sprite("sustainable_immortality.png", SUSTAINABLE_IMMORTALITY);
        if let frost::Shape::Sprite { alpha, .. } = &mut immortality {
            *alpha = IMMORTALITY_ALPHA;
        }
        Assets {
            grass: sprite("grass.png", GRASS),
            can: sprite("water_can_outline.png", WATER_CAN_OUTLINE),
            spray1: sprite("Spray1.png", SPRAY1),
            spray2: sprite("Spray2.png", SPRAY2),
            viper1: sprite("Getingeye1.png", GETINGEYE1),
            viper2: sprite("Getingeye2.png", GETINGEYE2),
            bug1: sprite("Bug1a.png", BUG1A),
            bug2: sprite("Bug2a.png", BUG2A),
            bug3: sprite("Bug3a.png", BUG3A),
            items: sprite("items.png", ITEMS),
            held_items: sprite("held_items.png", HELD_ITEMS),
            plant1: sprite("plant1.png", PLANT1),
            plant2: sprite("plant2.png", PLANT2),
            plant3: sprite("plant3.png", PLANT3),
            plant4: sprite("plant4.png", PLANT4),
            plant5: sprite("plant5.png", PLANT5),
            flower: sprite("flower.png", FLOWER),
            tomato: sprite("tomato.png", TOMATO),
            tomato_fg: sprite("tomato_fg.png", TOMATO_FG),
            basket_back: sprite("BasketBack.png", BASKET_BACK),
            basket_front: sprite("BasketFront.png", BASKET_FRONT),
            immortality,
            sounds: Sounds {
                device: audio,
                tjatters: [
                    sound("TjatterLow.wav", TJATTER_LOW),
                    sound("TjatterMid.wav", TJATTER_MID),
                    sound("TjatterHigh.wav", TJATTER_HIGH),
                ],
                plops: [
                    sound("bugs_plopp1.wav", BUGS_PLOPP1),
                    sound("bugs_plopp2.wav", BUGS_PLOPP2),
                    sound("bugs_plopp3.wav", BUGS_PLOPP3),
                ],
                ajs: [
                    sound("Aj1.wav", AJ1),
                    sound("Aj2.wav", AJ2),
                    sound("Aj3.wav", AJ3),
                    sound("Aj4.wav", AJ4),
                ],
                death: sound("bugsDeath.wav", BUGS_DEATH),
                pour: sound("WaterFlowSoft.wav", WATER_FLOW_SOFT),
                spray: sound("Spray.wav", SPRAY),
                tomato_drop: sound("tomatoDrop.wav", TOMATO_DROP),
            },
        }
    }
}

/// Decodes the embedded sprite `assets/sprites/{name}`, panicking with
/// the asset's path on failure.
fn sprite(name: &str, bytes: &[u8]) -> frost::Shape {
    frost::Shape::sprite_bytes(bytes)
        .unwrap_or_else(|err| panic!("failed to decode assets/sprites/{name}: {err}"))
}

/// Decodes the embedded sound `assets/audio/{name}`, panicking with the
/// asset's path on failure.
fn sound(name: &str, bytes: &[u8]) -> frost::Sound {
    frost::Sound::load_bytes(bytes)
        .unwrap_or_else(|err| panic!("failed to decode assets/audio/{name}: {err}"))
}
