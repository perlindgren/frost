# `dogs_name`

The monster art lives here: four whole-frame flipbook pages and the
seven parts the same monster was cut into, all in `assets/sprites`
as 8-bit RGBA PNGs.

| Example | What it shows | Run with |
|---|---|---|
| `dogs_name` | whole-image sprite animation: the four frames are loaded once at startup and swapped into one sprite node every 1/8 s (8 fps, wrapping forever) — an `Arc` clone per frame, not a re-decode. The cycle is a 1-2-1-4 gait: `Monster3.png` is an intentional duplicate of `Monster1.png`, the stand pose being the gait's two "up" beats between the half-squat (frame 2) and the deep crouch (frame 4). The window opens at a true physical 1920x1080 on any display via `Config::window_size_px` (960x540 points on a 2x Retina screen), the fit scale is computed from the live window size each frame, and a gentle float, sway, and breathing keeps the monster from sitting dead still on top of the art's own squash | `cargo run --example dogs_name` |
| `centipede` | a whole animal assembled from the cut-out parts and walked **seen from above**, driven by a procedurally generated gait: three chevron segments picked at random, the skull, the tail, and one whole leg out of each segment on both sides — a row along the top of the body and a row along the bottom — each leg a single sprite turned about its hip. It crawls right to left; once the tail tip has cleared the left edge it comes back in from the right with **one more segment**, grown from small to full size, so it comes back one segment longer every trip. Feet are *planted* — a stance covers exactly the ground the body travels under it — a swing bows outward instead of up, because from above that is the only trace a raised foot leaves, and the steps run head to tail in a metachronal wave on the same wavelength as the sideways ripple the body is riding. `rig.rs` holds all of it as pure geometry, so nothing about the walk depends on the renderer | `cargo run --example centipede` |

Because the folder is not named `main.rs`, both examples are declared
explicitly in `Cargo.toml` (cargo only auto-discovers `examples/*.rs` and
`examples/*/main.rs`). `centipede.rs` also has a side module, `rig.rs`,
beside it — an example's own folder is a module path, like `immortal`'s.

## The parts

Cropped tight to their own pixels, each with a three-texel transparent
margin so the sprite's soft edge is not shaved off by linear filtering.
They are the pieces of the posed monster, drawn to be assembled facing
left, and `centipede` assembles them:

| Part | Size | Used for | Joint it is hung from |
|---|---|---|---|
| `Monster_head.png` | 702×955 | the skull, eyes forward | the neck, mid-height on its rear edge |
| `Monster_body1.png` | 495×773 | a segment (the largest chevron) | its apex, which rides the spine |
| `Monster_body2.png` | 450×695 | a segment | its apex |
| `Monster_body3.png` | 397×717 | a segment (the smallest chevron) | its apex |
| `Monster_tail.png` | 322×268 | the rear spike cluster | its root, mid-height on the left |
| `Monster_leg1.png` | 347×918 | one whole leg, hip bar to clay foot | the hip, inside the bar |

There is one leg file because there is one leg in the art, and it was in
two: the original sheet's leg files were byte-identical rasters, each
holding the thigh in its top rows and the shin in its bottom ones with a
hundred rows of the middle hidden behind a body chevron. The two pieces are
welded here tip to tip — the row where each tapers to its broken end, lined
up on that row's centroid — into one complete leg, and `Monster_leg2.png`
went away with the duplicate. The join lands where the limb is at its
narrowest, so it reads as a knee; it is the same join the two-sprite rig
used to make every frame by pivoting thigh and shin at those tips.

## Patterns it shows

- **Flipbook by shape swap.** The frames live in the `Process`, not the
  scene; each frame tick assigns `node.shape = Some(frames[i].clone())`.
  Sprite pixels are shared behind an `Arc`, so the swap is cheap.
- **Fit scale from the art.** `(window / texture).min()` on the real
  texture size keeps full-frame art inside the window whatever its
  resolution, and the idle breathing multiplies that same fit scale.
- **Parts hung from joints.** A sprite is centred on its node, so a part
  is placed by translating the *joint* into place: `Transform::anchor`
  turns a texture pixel into that joint's local offset, and the node's
  transform becomes `translate(-pivot ⊙ scale) → rotate(angle) →
  translate(joint)`. The scale has to multiply the pivot too, because
  frost applies a node's scale before its transform.
- **A pose as pure geometry.** `rig::pose(rig, segments, t, &mut out)`
  fills a `Vec<Placement>` — part index, joint, tilt, scale — back to front.
  The example only turns placements into node transforms, which keeps the
  whole gait testable without a window, a device, or a scene.
- **Legs that grip, and drive.** A leg is rigid — one sprite, one pivot —
  so its foot can only ever sit on a circle of its own length around the
  hip, and that is the gait rather than a limit on it. The stride is set to
  `speed × duty × cycle`, so a planted foot stays exactly where it was put
  while the body moves on; how far *out* the foot stands is then not a
  choice but `√(length² − travel²)`, which is why the leg sweeps fast under
  the hip and crawls at the ends of the stance. A swinging leg is drawn
  shortened instead of raised, foreshortening being the only way a foot
  lifted off the ground shows up in a picture taken from directly above. And
  since a foot can only be dragged along the ground as far as its leg is
  long, these short legs are what set the fast step cycle — the cycle follows
  from the speed and the stride, rather than being an unrelated beat the legs
  have to dance to.
- **Rebuilding a subtree, not a scene.** The animal's node is rebuilt only
  when its body changes; every other frame just writes transforms into the
  nodes that already exist. Sprite pixels ride along as `Arc` clones, so
  even the rebuild is cheap.
- **Both sides from one leg.** There is one leg in the rig, not two. It is
  solved with its foot out on the `+y` side of its hip, and a `side` of
  `-1` mirrors the result: the joint's `y`, the two angles, and the sign of
  the node's `y` scale all turn with it. `Transform::scale` is a plain
  diagonal matrix and the sprite pipeline culls no faces, so a negative
  `y` mirrors the clay cleanly — the row of legs on the other side of the
  body costs one float, and the two sides step half a cycle apart.
