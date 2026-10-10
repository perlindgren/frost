//! The bench's data model: sprites, the world snapshot, and the
//! undo history.
// The subjects this one reads.
use crate::art::*;
use crate::desk::*;
use crate::map::*;
use crate::sidecar::*;
use frost::ron as ron_tree;

/// One sprite as pure data: everything its GPU shapes are derived from.
/// The shapes themselves never enter the history — a restored world
/// rebuilds them from these pixels.
#[derive(Clone)]
pub(crate) struct WorldSprite {
    pub(crate) path: std::path::PathBuf,
    pub(crate) name: String,
    pub(crate) current: image::RgbaImage,
    pub(crate) saved_img: image::RgbaImage,
    pub(crate) ron: Option<RonDoc>,
    pub(crate) saved_ron: Option<String>,
    pub(crate) atlas: Option<(usize, usize)>,
    pub(crate) thumb_img: image::RgbaImage,
}

/// One frame as data: the layers' PNG bytes (a layer's shape IS its
/// snapshot) and the time the frame holds.
#[derive(Clone)]
pub(crate) struct WorldFrame {
    pub(crate) layers: Vec<Vec<u8>>,
    pub(crate) next_time: f32,
}

/// The animation as data: the frames, the one the clock stands on, and
/// whether playback wraps. The running clock itself — playing, dir,
/// elapsed — stands outside history: undo does not pause a tune.
#[derive(Clone)]
pub(crate) struct WorldAnim {
    pub(crate) frames: Vec<WorldFrame>,
    pub(crate) frame: usize,
    pub(crate) looping: bool,
}

/// Everything the commands touch, captured whole: the unit of undo.
/// View settings (zoom, the greys), the dialogs' remembered folder and
/// the status line stand outside — history is about the bench's
/// content, not the way it is held.
#[derive(Clone)]
pub(crate) struct World {
    pub(crate) sprites: Vec<WorldSprite>,
    pub(crate) active: usize,
    pub(crate) anim: WorldAnim,
    pub(crate) view: View,
    pub(crate) selection: Option<[f32; 4]>,
    pub(crate) maps: Vec<MapLayer>,
}

/// One mouse-held paint stroke. `touched` gates the undo step: a
/// stroke that changes nothing (a click outside the map) must not
/// claim a place on the road.
#[derive(Clone, Copy)]
pub(crate) struct Paint {
    pub(crate) erase: bool,
    pub(crate) touched: bool,
    pub(crate) n: u32,
    /// Opened by the Delete key rather than a button; such a stroke
    /// ends when the key lets go, not on a mouse release.
    pub(crate) key_opened: bool,
}

/// The most worlds either road keeps; older steps age off the back.
pub(crate) const HISTORY_MAX: usize = 64;

/// One command's worth of memory: the world as it stood goes on the
/// undo road, and the redo road — the future this step chose over —
/// dissolves.
pub(crate) fn push_step(undo: &mut Vec<World>, redo: &mut Vec<World>, now: World) {
    undo.push(now);
    if undo.len() > HISTORY_MAX {
        undo.remove(0);
    }
    redo.clear();
}

/// One step back: the newest past becomes the present, and the present
/// goes onto the road forward. `None` when there is no past.
pub(crate) fn step_back(undo: &mut Vec<World>, redo: &mut Vec<World>, now: World) -> Option<World> {
    let past = undo.pop()?;
    redo.push(now);
    Some(past)
}

/// One step forward: the mirror image.
pub(crate) fn step_forward(
    undo: &mut Vec<World>,
    redo: &mut Vec<World>,
    now: World,
) -> Option<World> {
    let future = redo.pop()?;
    undo.push(now);
    Some(future)
}

/// The time a new frame holds before the next one, in seconds; the
/// duration slider adjusts it per frame.
pub(crate) const TIME_DEFAULT: f32 = 0.2;

/// The frame duration slider's range, in seconds till the next frame.
pub(crate) const TIME_MIN: f32 = 0.02;

pub(crate) const TIME_MAX: f32 = 2.0;

/// One sprite: its files, its textures and its slot's look.
pub(crate) struct Sprite {
    /// The file the sprite was loaded from — Save writes back to it.
    pub(crate) path: std::path::PathBuf,
    /// The file's name, for the HUD, the dialogs and the logs.
    pub(crate) name: String,
    /// The working texture: crops replace it, saves write it.
    pub(crate) current: image::RgbaImage,
    /// The texture as it stands on the file: the loader's decode, then
    /// each save's bytes. The pixels compare against this to answer
    /// whether Save has anything to do — undoing back to the loaded
    /// picture is not a change, and an unchanged PNG is never re-encoded
    /// (its file keeps the container that produced it).
    pub(crate) saved_img: image::RgbaImage,
    /// The small picture a slot shows — kept as pixels, because when a
    /// sprite returns through undo its thumbnail's shape is rebuilt
    /// from these.
    pub(crate) thumb_img: image::RgbaImage,
    /// The working texture as a shape — the work area's sprite node.
    pub(crate) shape: frost::Shape,
    /// The original, minimized to a slot's width — the slot's picture.
    /// It never changes: a slot shows the sprite as it was loaded.
    pub(crate) thumb: frost::Shape,
    /// The parsed sidecar `<name>.ron`, `None` when the sprite has none
    /// or the file does not parse. It lives and dies with the sprite:
    /// closing the slot drops the panel with it.
    pub(crate) ron: Option<RonDoc>,
    /// The sidecar as it stands on the file, in the canonical text Save
    /// writes — `None` when the sprite loaded without one. The tree's
    /// counterpart of `saved_img`: same question, same answer.
    pub(crate) saved_ron: Option<String>,
    /// The tile grid the sprite splits into: the rows and columns the
    /// Atlas panel sets, read back from the sidecar's `atlas` field when
    /// the sprite loads. `None` is no grid. Saving a sprite that names a
    /// grid but has no sidecar yet creates one beside the PNG, so the
    /// grid has a file to live in.
    pub(crate) atlas: Option<(usize, usize)>,
}

impl Sprite {
    /// Whether the texture differs from the file's bytes: Save's question.
    pub(crate) fn png_changed(&self) -> bool {
        self.current != self.saved_img
    }

    /// The sidecar text a save would write: the parsed tree's rendering,
    /// or — for a sprite that names a grid but has no sidecar yet — the
    /// new file's text. `None` is "a save writes no sidecar".
    pub(crate) fn ron_text(&self) -> Option<String> {
        match &self.ron {
            Some(doc) => Some(ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer)),
            None => self.atlas.map(|atlas| {
                let doc = new_sidecar(file_name_of(&self.path.with_extension("ron")), atlas);
                ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer)
            }),
        }
    }

    /// Whether the sidecar — file or file-to-be — differs from the disk:
    /// the other question Save asks.
    pub(crate) fn ron_changed(&self) -> bool {
        self.ron_text()
            .is_some_and(|text| Some(&text) != self.saved_ron.as_ref())
    }

    /// Whether either save target differs from its file: the gate on the
    /// Save button, the `*` marker and the overwrite prompt.
    pub(crate) fn changed(&self) -> bool {
        self.png_changed() || self.ron_changed()
    }
}

/// One picture within a frame: the active sprite's shape, snapshotted
/// when the layer was added — later crops and closed slots leave the
/// frame's layers untouched. The PNG the shape was built from rides
/// along: the history rebuilds a layer's shape from these bytes.
pub(crate) struct Layer {
    pub(crate) shape: frost::Shape,
    pub(crate) png: Vec<u8>,
}

/// One animation frame: a set of layers, stacked as added, and the time
/// it holds before the next frame.
#[derive(Default)]
pub(crate) struct Frame {
    pub(crate) layers: Vec<Layer>,
    pub(crate) next_time: f32,
}

/// The animation: an ordered set of frames kept by a clock. Looping
/// playback wraps at the ends; ping-pong walks back through the frames
/// instead, each end held once per sweep.
#[derive(Default)]
pub(crate) struct Anim {
    pub(crate) frames: Vec<Frame>,
    /// The frame the clock is in — also the frame the panel edits.
    pub(crate) frame: usize,
    /// Whether playback wraps at the ends (loop) or walks back
    /// (ping-pong).
    pub(crate) looping: bool,
    /// Whether the clock runs.
    pub(crate) playing: bool,
    /// The clock's direction: `1` forward, `-1` back (ping-pong only).
    pub(crate) dir: i32,
    /// Seconds spent in the current frame.
    pub(crate) elapsed: f32,
}

impl Anim {
    /// Jump to a frame and restart its hold: the panel's prev/next.
    pub(crate) fn set_frame(&mut self, frame: usize) {
        self.frame = frame;
        self.elapsed = 0.0;
    }

    /// Run the clock for `dt` seconds; whether the shown frame changed.
    pub(crate) fn tick(&mut self, dt: f32) -> bool {
        if !self.playing || self.frames.len() < 2 {
            return false;
        }
        self.elapsed += dt;
        let mut moved = false;
        // A slow frame can spend several holds at once: consume them one
        // by one, with a guard against a zero-length run of frames.
        let mut guard = 0;
        while self.elapsed >= self.frames[self.frame].next_time && guard < 512 {
            self.elapsed -= self.frames[self.frame].next_time.max(1e-4);
            self.step();
            moved = true;
            guard += 1;
        }
        moved
    }

    /// One step along the play direction: looping always wraps forward;
    /// ping-pong walks back and forth, each end held once per sweep.
    pub(crate) fn step(&mut self) {
        let n = self.frames.len();
        if self.looping {
            self.dir = 1;
            self.frame = (self.frame + 1) % n;
            return;
        }
        if self.dir > 0 {
            if self.frame + 1 < n {
                self.frame += 1;
            } else {
                self.dir = -1;
                self.frame = n - 2; // `n >= 2`: bounce off the last frame
            }
        } else if self.frame > 0 {
            self.frame -= 1;
        } else {
            self.dir = 1;
            self.frame = 1.min(n - 1); // bounce off the first frame
        }
    }
}

// A layer-less frame with a chosen hold time, for the clock tests.
#[cfg(test)]
pub(crate) fn frame(next_time: f32) -> Frame {
    Frame {
        next_time,
        ..Frame::default()
    }
}

#[cfg(test)]
pub(crate) fn anim(times: &[f32], looping: bool) -> Anim {
    Anim {
        frames: times.iter().map(|&t| frame(t)).collect(),
        looping,
        playing: true,
        dir: 1,
        ..Anim::default()
    }
}

#[cfg(test)]
pub(crate) fn bench(active: usize) -> World {
    World {
        sprites: Vec::new(),
        active,
        anim: WorldAnim {
            frames: Vec::new(),
            frame: 0,
            looping: true,
        },
        view: View::Markers,
        selection: None,
        maps: Vec::new(),
    }
}

/// A stub sprite: a 4 x 2-pixel texture split 2 x 2, every cell a
/// 2 x 1 rectangle — the sizes the layer tests quote.
#[cfg(test)]
pub(crate) fn tileset(name: &str) -> Sprite {
    let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([9, 9, 9, 255]));
    let png = png_bytes(&img).unwrap();
    Sprite {
        path: std::path::PathBuf::from(name),
        name: name.to_string(),
        current: img.clone(),
        saved_img: img.clone(),
        thumb_img: img,
        shape: frost::Shape::sprite_bytes_nearest(&png).unwrap(),
        thumb: frost::Shape::sprite_bytes_nearest(&png).unwrap(),
        ron: None,
        saved_ron: None,
        atlas: Some((2, 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_holds_each_frame_for_its_time() {
        // Two frames: 0.1 s then 0.2 s. The clock steps only once the
        // current frame's hold is spent, carrying the remainder over.
        let mut a = anim(&[0.1, 0.2], true);
        assert!(!a.tick(0.05)); // mid-frame 0
        assert_eq!(a.frame, 0);
        assert!(a.tick(0.07)); // 0.12 >= 0.1 -> frame 1, 0.02 over
        assert_eq!(a.frame, 1);
        assert!(!a.tick(0.1)); // 0.12 < 0.2 -> still frame 1
        assert_eq!(a.frame, 1);
        assert!(a.tick(0.12)); // 0.24 >= 0.2 -> step, looping back to 0
        assert_eq!(a.frame, 0);
    }

    #[test]
    fn loop_wraps_while_ping_pong_bounces() {
        // Looping: 0 1 2, 0 1 2, …
        let mut a = anim(&[1.0, 1.0, 1.0], true);
        let mut seq = Vec::new();
        for _ in 0..7 {
            seq.push(a.frame);
            a.step();
        }
        assert_eq!(seq, vec![0, 1, 2, 0, 1, 2, 0]);
        // Ping-pong: 0 1 2 1, 0 1 2 1, … — each end held once per sweep.
        let mut b = anim(&[1.0, 1.0, 1.0], false);
        let mut seq = Vec::new();
        for _ in 0..9 {
            seq.push(b.frame);
            b.step();
        }
        assert_eq!(seq, vec![0, 1, 2, 1, 0, 1, 2, 1, 0]);
    }

    #[test]
    fn a_single_frame_animation_never_steps() {
        let mut a = anim(&[0.1], true);
        assert!(!a.tick(1.0));
        assert_eq!(a.frame, 0);
        // Two frames a beat apart step cleanly.
        let mut c = anim(&[0.5, 0.5], false);
        assert!(!c.tick(0.4));
        assert!(c.tick(0.2)); // 0.6 >= 0.5 -> frame 1, 0.1 over
        assert_eq!(c.frame, 1);
    }

    #[test]
    fn the_history_walks_back_and_forth() {
        // The roads hold the before-pictures of commands. The bench sat
        // on slot 0, a command moved it to 1 (world 0 went down), a
        // second moved it to 2 (world 1 went down): the present is 2.
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        push_step(&mut undo, &mut redo, bench(0));
        push_step(&mut undo, &mut redo, bench(1));
        // One step back: world 1 emerges, the present goes to redo.
        let past = step_back(&mut undo, &mut redo, bench(2)).expect("one past");
        assert_eq!(past.active, 1);
        assert_eq!(redo.len(), 1);
        // One more: world 0, the bench's first seat.
        let older = step_back(&mut undo, &mut redo, past).expect("two pasts");
        assert_eq!(older.active, 0);
        // And forward again, the same worlds in the same order.
        let back1 = step_forward(&mut undo, &mut redo, older).expect("one future");
        assert_eq!(back1.active, 1);
        let back2 = step_forward(&mut undo, &mut redo, back1).expect("two futures");
        assert_eq!(back2.active, 2);
        // Walked dry, forward is a polite nothing.
        assert!(step_forward(&mut undo, &mut redo, back2).is_none());
    }

    #[test]
    fn a_new_command_dissolves_the_redo_road() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        push_step(&mut undo, &mut redo, bench(0));
        step_back(&mut undo, &mut redo, bench(1));
        assert_eq!(redo.len(), 1);
        // A fresh command: the future the undo had opened is gone.
        push_step(&mut undo, &mut redo, bench(2));
        assert!(redo.is_empty());
    }

    #[test]
    fn the_history_ages_its_oldest_steps_out() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        for i in 0..(HISTORY_MAX + 10) {
            push_step(&mut undo, &mut redo, bench(i % 7));
        }
        assert_eq!(undo.len(), HISTORY_MAX);
        assert_eq!(
            undo.last().expect("not empty").active,
            (HISTORY_MAX + 9) % 7
        );
    }
}
