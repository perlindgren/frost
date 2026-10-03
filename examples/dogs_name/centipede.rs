//! A centipede built out of the `dogs_name` monster's cut-out parts, and
//! the walk those parts were cut to make.
//!
//! [`Monster_head.png`](assets/sprites/Monster_head.png), the three
//! [`Monster_body`](assets/sprites/Monster_body1.png) chevrons,
//! [`Monster_tail.png`](assets/sprites/Monster_tail.png) and the two halves
//! of [`Monster_leg1.png`](assets/sprites/Monster_leg1.png) and
//! [`Monster_leg2.png`](assets/sprites/Monster_leg2.png) are loaded once and
//! assembled by the [`rig`] module into an animal with a two-link leg on
//! every segment: the head at the front, a chevron per segment with its
//! knee hidden behind the paper hem as it is in the sheet, the spike cluster
//! at the rear, and thigh-and-shin hanging under each segment.
//!
//! The animal starts as three segments drawn at random from the three
//! chevron variants, crawls right to left at [`rig::CRAWL_SPEED`], and when
//! the whole of it — tail last — has gone off the left edge it comes back in
//! from the right with **one more segment**. The newcomer is grown, not
//! pasted on: it is a smaller chevron with shorter legs that reaches full
//! size over [`rig::GROW_SECONDS`], so the addition reads as an animal
//! moulting and stretching rather than a longer list of sprites. One more
//! segment every round trip, until [`MAX_SEGMENTS`] makes it longer than the
//! screen is wide and the crawl simply continues at that length.
//!
//! The gait is [`rig`]’s: every segment’s foot alternates between a stance
//! planted on the ground and a swing that reaches forward, each leg’s step
//! [`rig::LAG`] of a cycle behind its neighbour’s, so the steps run head to
//! tail in the same ripple the body’s spine is riding. Two details do most
//! of the work of selling it:
//!
//! - the stride is exactly the ground the animal travels while a foot is
//!   planted, so planted feet *grip* — the body slides over them instead of
//!   the whole animal skating along on shuffling feet, and
//! - the far-side legs are a half cycle out of step with the near ones,
//!   shaded and slightly smaller, which is how one flat row of sprites reads
//!   as two rows of legs under an animal.
//!
//! None of it is keyframed: the whole pose is a pure function of the clock,
//! so a segment can be added mid-stride and nothing has to be re-tuned.
//!
//! The window opens at a physical 1920x1080 like `dogs_name`
//! ([`frost::Config::window_size_px`]); the animal is scaled to the live
//! window each frame and walks along a soil band under the same moonlit
//! grey-green the flipbook sits in. Run with:
//!
//! ```text
//! cargo run --example centipede
//! ```

mod rig;

use rig::{Placement, Segment};

/// The window's initial size in physical pixels, as in `dogs_name`: a real
/// 1920x1080 panel window on every display, so the parts land one texel per
/// pixel there and the fit scale below only has to cope with smaller
/// windows.
const WINDOW: [u32; 2] = [1920, 1080];

/// How many segments the animal is born with. Each trip across the window
/// adds one.
const START_SEGMENTS: usize = 3;

/// The longest the animal grows. A centipede is longer than the window long
/// before this, so the cap exists only to stop a very patient viewer from
/// growing the scene forever: at the cap the animal holds its length and
/// keeps crawling.
const MAX_SEGMENTS: usize = 26;

/// The animal's height as a fraction of the window's. The fit scale maps
/// the animal's art-space height — a hip at [`rig::Rig::spine_y`] and the
/// skull above it — onto this, leaving sky over it and soil under it.
const BODY_HEIGHT: f32 = 0.44;

/// The ground line, as a fraction of the window's height below its centre:
/// the animal walks along the lower third of the frame.
const GROUND: f32 = 0.30;

/// How far past an edge the animal must be before it comes back at the
/// other one, measured from its nose and its tail tip rather than from its
/// origin, so it really has left the scene instead of respawning half-in.
const MARGIN: f32 = 60.0;

/// The root's children, in the order that draws them: the soil band, its
/// lit edge along the ground line, and the animal on top of both.
const SOIL: usize = 0;
const EDGE: usize = 1;
const ANIMAL: usize = 2;

/// The centipede: the segments it is built from, the clock its gait reads,
/// and where it stands.
struct Centipede {
    /// The clock in seconds. The pose is a pure function of it, so nothing
    /// else in here is animation state.
    t: f32,
    /// The body, head first: a chevron variant and how far that segment has
    /// grown, `1.0` being full size.
    segments: Vec<Segment>,
    /// Where the animal's origin sits in window pixels: the middle of its
    /// front segment horizontally, and the ground it walks on vertically.
    x: f32,
    ground_y: f32,
    /// The rig's numbers, derived once from the parts' texture sizes.
    rig: rig::Rig,
    /// The parts, indexed by [`rig`] part index.
    shapes: Vec<frost::Shape>,
    /// The scratch buffer [`rig::pose`] writes into, kept out of the scene.
    placements: Vec<Placement>,
    /// The source of the segments it picks, and of the order it picks them.
    rng: frost::Rng,
    /// Whether the animal's nodes have to be rebuilt: set whenever its body
    /// changes, which is not only when it changes length.
    rebuild: bool,
}

impl Centipede {
    /// One of the three chevron variants, at random.
    fn random_body(&mut self) -> usize {
        let n = rig::BODIES.len() as u64;
        rig::BODIES[(self.rng.next_u64() % n) as usize]
    }

    /// How far the animal's origin is from its two ends in art units: the
    /// nose in front of it (negative) and the tail tip behind it. The
    /// wrap-around test needs both, because the origin is a point on the
    /// body and neither end is at it.
    fn front(&self) -> f32 {
        rig::HEAD_AT * rig::PITCH - self.rig.nose
    }

    fn rear(&self) -> f32 {
        rig::rear(&self.rig, &self.segments)
    }

    /// The fit scale: the animal's art-space height — a hip at
    /// [`rig::Rig::spine_y`] with the skull above it — onto [`BODY_HEIGHT`]
    /// of the window, uniform: a centipede stretched sideways is just a
    /// wrong centipede.
    fn fit(&self, height: f32) -> f32 {
        BODY_HEIGHT * height / (self.rig.spine_y() + self.rig.skull)
    }

    /// Starts a fresh animal of `count` segments, entering from the right
    /// edge with its nose just off screen. The last segment is the new one,
    /// and starts at [`rig::GROW_START`]; the ones in front of it are full
    /// size, as they were in the animal that just left.
    fn respawn(&mut self, count: usize, width: f32, fit: f32) {
        self.segments.clear();
        for _ in 0..count {
            let body = self.random_body();
            self.segments.push(Segment { body, grow: 1.0 });
        }
        if let Some(newest) = self.segments.last_mut() {
            newest.grow = rig::GROW_START;
        }
        // The nose at `width / 2 + MARGIN`, and the rest of the animal
        // trailing off the edge behind it.
        self.x = width / 2.0 + MARGIN - self.front() * fit;
        // Even at `MAX_SEGMENTS`, where the length stops changing, this is a
        // different animal: it picked a fresh set of chevron variants.
        self.rebuild = true;
    }
}

impl frost::Process for Centipede {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;
        let (width, height) = ctx.size();
        let fit = self.fit(height);
        self.ground_y = -GROUND * height;

        // The first frame knows the window size, which is when the animal
        // can be placed at all: build it entering from the right.
        if self.segments.is_empty() {
            self.respawn(START_SEGMENTS, width, fit);
        }

        // Every segment stretches toward full size; only a newborn is ever
        // short of it, and it gets there in `GROW_SECONDS`.
        for segment in &mut self.segments {
            segment.grow = (segment.grow + dt / rig::GROW_SECONDS).min(1.0);
        }

        // The crawl: forward is `-x`, at the speed the feet are strided for.
        self.x -= rig::CRAWL_SPEED * fit * dt;

        // One trip across the window is one round trip of the gait's clock
        // only in a loose sense — what matters is that the animal is gone.
        // The tail is the last of it to leave, so when the tail tip passes
        // the left margin it is time to come back with a new segment.
        if self.x + self.rear() * fit < -width / 2.0 - MARGIN {
            let grown = (self.segments.len() + 1).min(MAX_SEGMENTS);
            self.respawn(grown, width, fit);
            log::info!("the centipede grew: {} segments", self.segments.len());
        }

        // The pose, in art units, back to front: far legs, near legs, tail,
        // segments, head.
        rig::pose(&self.rig, &self.segments, self.t, &mut self.placements);

        // The soil: a band from the ground line to the bottom of the window,
        // with a lit edge along the line itself. Both are the root's
        // children in window space, so they follow the window's size.
        let soil = &mut ctx.scene().root.children[SOIL];
        let depth = height / 2.0 + self.ground_y;
        soil.shape = Some(frost::Shape::Rectangle {
            center: [0.0, self.ground_y - depth / 2.0],
            extent: [width / 2.0, depth / 2.0],
            color: frost::Color {
                r: 0.045,
                g: 0.055,
                b: 0.05,
                a: 1.0,
            },
        });
        let edge = &mut ctx.scene().root.children[EDGE];
        edge.shape = Some(frost::Shape::Rectangle {
            center: [0.0, self.ground_y],
            extent: [width / 2.0, 1.5],
            color: frost::Color {
                r: 0.16,
                g: 0.2,
                b: 0.16,
                a: 1.0,
            },
        });

        // The animal: one node per placement. The node carries the fit scale
        // and the animal's place, so a placement's own transform stays in
        // art units and only has to hang its part from its joint.
        let animal = &mut ctx.scene().root.children[ANIMAL];
        animal.transform = frost::Transform::translate([self.x, self.ground_y]);
        animal.scale = [fit, fit];

        // The tree is rebuilt only when the body changes — which the rig
        // reports as a placement count, plus the one case the count cannot
        // see: an animal at the length cap comes back just as long, but out
        // of a different set of chevron variants.
        if self.rebuild || animal.children.len() != self.placements.len() {
            self.rebuild = false;
            animal.children = self
                .placements
                .iter()
                .map(|p| {
                    Box::new(frost::SceneNode {
                        shape: Some(self.shapes[p.part].clone()),
                        // The far side's legs are the same clay in shadow.
                        modulate: frost::Color {
                            r: p.shade,
                            g: p.shade,
                            b: p.shade,
                            a: 1.0,
                        },
                        ..Default::default()
                    })
                })
                .collect();
        }

        for (node, p) in animal.children.iter_mut().zip(&self.placements) {
            // A part is centred on its node, so its joint has to be moved
            // back onto the node's origin — and since frost applies a node's
            // scale *before* its transform, that offset has to be scaled by
            // the same factor to keep the joint where the rig put it.
            node.transform =
                frost::Transform::translate([-p.pivot[0] * p.scale, -p.pivot[1] * p.scale])
                    .compose(&frost::Transform::rotate(p.angle))
                    .compose(&frost::Transform::translate(p.joint));
            node.scale = [p.scale, p.scale];
        }

        log::trace!(
            "process: dt {dt:?}, {} segments at x {:.0}, {} draws",
            self.segments.len(),
            self.x,
            self.placements.len()
        );
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let shapes: Vec<frost::Shape> = rig::PART_FILES
        .iter()
        .map(|file| {
            frost::Shape::sprite(format!("{root}/examples/dogs_name/assets/sprites/{file}"))
                .unwrap_or_else(|err| panic!("failed to load {file}: {err}"))
        })
        .collect();

    // The rig reads the parts' real texture sizes, so the joints in `rig`
    // can stay in image pixels.
    let mut sizes = [[0.0f32; 2]; rig::PART_COUNT];
    for (slot, shape) in sizes.iter_mut().zip(&shapes) {
        *slot = shape
            .sprite_size()
            .expect("every part is loaded as a sprite");
    }

    let scene = frost::Scene::new(frost::SceneNode {
        // The same moonlit grey-green night the flipbook sits in.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.07,
                g: 0.09,
                b: 0.11,
                a: 1.0,
            },
        }),
        children: vec![
            // The soil band and its lit edge, sized by the process, and the
            // animal's node, built by the process. All three start empty:
            // their shapes and children are the process's business, because
            // only it knows the window size.
            Box::new(frost::SceneNode::default()),
            Box::new(frost::SceneNode::default()),
            Box::new(frost::SceneNode::default()),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Centipede {
            t: 0.0,
            segments: Vec::new(),
            x: 0.0,
            ground_y: 0.0,
            rig: rig::Rig::new(sizes),
            shapes,
            placements: Vec::new(),
            rng: frost::Rng::new(),
            rebuild: true,
        },
        frost::Config {
            vsync: true,
            window_size: None,
            window_size_px: Some(WINDOW),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
