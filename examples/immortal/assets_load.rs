//! Startup asset loading for the `immortal` example: [`Assets::load`]
//! reads everything the example loads at startup, from the asset root —
//! the sprite shapes, the audio output, and the bug and watering clips —
//! in one place, and `main` hands it to the plant node, the scene, and
//! the `Demo`.

/// The badge's tint alpha: a faint watermark behind the play field.
const IMMORTALITY_ALPHA: f32 = 0.2;

/// Everything the example loads at startup, from the asset root: the
/// sprite shapes, the audio output, and the bug and watering clips.
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
    /// The audio output, opened once at startup.
    pub audio: frost::Audio,
    /// The tjatter clips: a bug that reaches its destination while another
    /// bug is on the grass within 50 px of it chatters on one of them, the
    /// swarm's random pick.
    pub tjatters: [frost::Sound; 3],
    /// The plopp clips: a bug that pops up out of the grass — a batch
    /// spawn or a respawn — plops on one of them, the swarm's random pick.
    pub plops: [frost::Sound; 3],
    /// The Aj clips: the bug cries out on one of them, the swarm's random
    /// pick, every time the mist drops its health.
    pub ajs: [frost::Sound; 4],
    /// The bug death clip, `assets/audio/bugsDeath.wav`, decoded once at
    /// startup.
    pub death: frost::Sound,
    /// The watering loop: it plays, looping, while water pours out of the
    /// spout, silenced the frame pouring stops.
    pub pour: frost::Sound,
    /// The spray hiss, `assets/audio/Spray.wav`, decoded once at startup.
    pub spray: frost::Sound,
}

impl Assets {
    /// Loads every sprite from `{root}/assets/sprites` and every sound
    /// from `{root}/assets/audio`, opening the audio output first. The
    /// immortality badge is tinted down to a faint watermark on the way
    /// in.
    pub fn load(root: &str) -> Self {
        let audio = frost::Audio::new().expect("failed to open the audio output device");
        let mut immortality = sprite(root, "sustainable_immortality.png");
        if let frost::Shape::Sprite { alpha, .. } = &mut immortality {
            *alpha = IMMORTALITY_ALPHA;
        }
        Assets {
            grass: sprite(root, "grass.png"),
            can: sprite(root, "water_can_outline.png"),
            spray1: sprite(root, "Spray1.png"),
            spray2: sprite(root, "Spray2.png"),
            viper1: sprite(root, "Getingeye1.png"),
            viper2: sprite(root, "Getingeye2.png"),
            bug1: sprite(root, "Bug1a.png"),
            bug2: sprite(root, "Bug2a.png"),
            bug3: sprite(root, "Bug3a.png"),
            items: sprite(root, "items.png"),
            held_items: sprite(root, "held_items.png"),
            plant1: sprite(root, "plant1.png"),
            plant2: sprite(root, "plant2.png"),
            plant3: sprite(root, "plant3.png"),
            plant4: sprite(root, "plant4.png"),
            plant5: sprite(root, "plant5.png"),
            flower: sprite(root, "flower.png"),
            tomato: sprite(root, "tomato.png"),
            tomato_fg: sprite(root, "tomato_fg.png"),
            basket_back: sprite(root, "BasketBack.png"),
            basket_front: sprite(root, "BasketFront.png"),
            immortality,
            audio,
            tjatters: [
                sound(root, "TjatterLow.wav"),
                sound(root, "TjatterMid.wav"),
                sound(root, "TjatterHigh.wav"),
            ],
            plops: [
                sound(root, "bugs_plopp1.wav"),
                sound(root, "bugs_plopp2.wav"),
                sound(root, "bugs_plopp3.wav"),
            ],
            ajs: [
                sound(root, "Aj1.wav"),
                sound(root, "Aj2.wav"),
                sound(root, "Aj3.wav"),
                sound(root, "Aj4.wav"),
            ],
            death: sound(root, "bugsDeath.wav"),
            pour: sound(root, "WaterFlowSoft.wav"),
            spray: sound(root, "Spray.wav"),
        }
    }
}

/// Loads the sprite `{root}/assets/sprites/{name}`, panicking with the
/// full path on failure.
fn sprite(root: &str, name: &str) -> frost::Shape {
    let path = format!("{root}/assets/sprites/{name}");
    frost::Shape::sprite(&path)
        .unwrap_or_else(|err| panic!("failed to load {path}: {err}"))
}

/// Loads the sound `{root}/assets/audio/{name}`, panicking with the full
/// path on failure.
fn sound(root: &str, name: &str) -> frost::Sound {
    let path = format!("{root}/assets/audio/{name}");
    frost::Sound::load(&path)
        .unwrap_or_else(|err| panic!("failed to load {path}: {err}"))
}
