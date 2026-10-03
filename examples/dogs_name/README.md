# `dogs_name`

The monster art lives here: four whole-frame flipbook pages and the
seven parts the same monster was cut into, all in `assets/sprites`
as 8-bit RGBA PNGs.

| Example | What it shows | Run with |
|---|---|---|
| `dogs_name` | whole-image sprite animation: the four frames are loaded once at startup and swapped into one sprite node every 1/8 s (8 fps, wrapping forever) — an `Arc` clone per frame, not a re-decode. The cycle is a 1-2-1-4 gait: `Monster3.png` is an intentional duplicate of `Monster1.png`, the stand pose being the gait's two "up" beats between the half-squat (frame 2) and the deep crouch (frame 4). The window opens at a true physical 1920x1080 on any display via `Config::window_size_px` (960x540 points on a 2x Retina screen), the fit scale is computed from the live window size each frame, and a gentle float, sway, and breathing keeps the monster from sitting dead still on top of the art's own squash | `cargo run --example dogs_name` |
| `centipede` | a whole animal assembled from the cut-out parts, and a procedurally generated gait: three chevron segments picked at random, a head, a tail, and a two-link leg under every segment whose foot is placed by inverse kinematics. It crawls right to left; once the tail tip has cleared the left edge it comes back in from the right with **one more segment**, grown from small to full size, so it comes back one segment longer every trip. Feet are *planted* — a stance covers exactly the ground the body travels under it — and the steps run head to tail in a metachronal wave that matches the ripple the spine is riding. `rig.rs` holds all of it as pure geometry, so nothing about the walk depends on the renderer | `cargo run --example centipede` |

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
| `Monster_leg1.png` | 250×448 | the hip bar and thigh | the hip, inside the bar |
| `Monster_leg2.png` | 320×474 | the shin and the clay foot | the knee, at its upper tip |

The two leg files were byte-identical rasters of a single leg, cut in two
exactly where a chevron covers its middle in the sheet — so they are
cropped as the two halves of that one leg: `leg1` keeps the hip bar and
thigh, `leg2` the shin and foot. Rotating each about its own tip is what
bends the knee.

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
  transform becomes `translate(-pivot · scale) → rotate(angle) →
  translate(joint)`. The scale has to multiply the pivot too, because
  frost applies a node's scale before its transform.
- **A pose as pure geometry.** `rig::pose(rig, segments, t, &mut out)`
  fills a `Vec<Placement>` — part index, joint, tilt, scale, shade — back
  to front. The example only turns placements into node transforms, which
  keeps the whole gait testable without a window, a device, or a scene.
- **Legs that grip.** Two-link inverse kinematics per segment, with the
  stride set to `speed × duty × cycle` so a planted foot stays exactly
  where it was put while the body moves on. Scaling the hip's height to
  `0.82` of the leg's own reach (`Rig::spine_y`) keeps a permanent bend in
  every knee, so a step has somewhere to bend further into.
- **Rebuilding a subtree, not a scene.** The animal's node is rebuilt only
  when its body changes; every other frame just writes transforms into the
  nodes that already exist. Sprite pixels ride along as `Arc` clones, so
  even the rebuild is cheap.
- **Depth from one sprite set.** The far-side legs are the same two parts
  with `modulate` at 45 %, drawn 9 % smaller, on a ground line slightly
  above the near one and a half cycle out of step — one flat row of sprites
  reads as two rows of legs.
