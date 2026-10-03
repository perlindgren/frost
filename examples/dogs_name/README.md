# `dogs_name`

The monster art lives here: four whole-frame flipbook pages and the
seven parts the same monster was cut into, all in `assets/sprites`
as 8-bit RGBA PNGs.

| Example | What it shows | Run with |
|---|---|---|
| `dogs_name` | whole-image sprite animation: the four frames are loaded once at startup and swapped into one sprite node every 1/8 s (8 fps, wrapping forever) — an `Arc` clone per frame, not a re-decode. The cycle is a 1-2-1-4 gait: `Monster3.png` is an intentional duplicate of `Monster1.png`, the stand pose being the gait's two "up" beats between the half-squat (frame 2) and the deep crouch (frame 4). The window opens at a true physical 1920x1080 on any display via `Config::window_size_px` (960x540 points on a 2x Retina screen), the fit scale is computed from the live window size each frame, and a gentle float, sway, and breathing keeps the monster from sitting dead still on top of the art's own squash | `cargo run --example dogs_name` |
| `centipede` | a whole animal assembled from the cut-out parts and walked **seen from above**, driven by a procedurally generated gait: three chevron segments picked at random, the skull, the tail, and a leg out of each segment on both sides — a row along the top of the body and a row along the bottom — each leg one sprite turned about one pivot. It crawls right to left and grows **one more segment while it is on screen**: once the whole animal is through the right edge, a plate is put on at the rear and slides out from under the last one, swelling as it goes and taking its legs out with it, while the tail — which lies over the rear of the body rather than being part of it — waits until the newcomer reaches past the plate it was resting on, and is then pushed along. One segment per crossing, so the growth is watched rather than missed off screen. Feet are *planted* — a stance covers exactly the ground the body travels under it — a swing is drawn as a shortened leg, because a foot lifted off the ground is nearer the eye and shorter in projection, the only trace a lift leaves from directly above, and the steps run head to tail in a metachronal wave on the same wavelength as the sideways ripple the body is riding. `rig.rs` holds all of it as pure geometry, so nothing about the walk depends on the renderer | `cargo run --example centipede` |

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
| `Monster_body1.png` | 495×773 | a segment (the largest chevron) | its apex fore-and-aft, the middle of the cut across |
| `Monster_body2.png` | 450×695 | a segment | the same, its own middle |
| `Monster_body3.png` | 397×717 | a segment (the smallest chevron) | the same, its own middle |
| `Monster_tail.png` | 322×268 | the rear spike cluster, drawn over the last segment | its root, mid-height on the left |
| `Monster_leg1.png` | 250×448 | the thigh — **not drawn**, see below | the hip, inside the bar |
| `Monster_leg2.png` | 320×474 | the leg: the shin and its clay foot | the shin's broken upper tip |

The leg is in two files because the limb in the sheet is in two pieces:
`leg1` holds the hip bar and thigh, `leg2` the shin and the clay foot, and
the hundred rows between them are where a body chevron covered the middle.
The two files used to be byte-identical rasters of that one leg. The
centipede draws one sprite per leg and takes the piece with the foot in it,
since a gait is something a foot does — swung from the fragment's broken
end, which is four pixels across and sits buried under a plate. That is also
why the legs read short: three fifths of the limb, and all of it inside the
body's own width. `Monster_leg1.png` stays in the folder as the other half
of the art, unused.

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
- **A segment arrives; it does not appear.** A newcomer is born on the slot of
  the segment in front of it, where the plate already there hides it — plates are
  emitted rear first, so the older plate is painted over the new one — and it
  slides back into a slot of its own across the same 1.2 s it spends swelling,
  every gap between segments being a fraction of `PITCH` until both have
  settled. The alternative, putting it straight into a pitch-sized slot, adds the
  whole length of a segment in one frame, and a viewer who saw nothing of the
  growth sees all of the hiccup. The tail is deliberately *not* part of this: it
  is drawn over the rear of the body — emitted after the plates, since in a 2D
  example the draw order is the only thing that puts one part on top of another —
  and it hangs off the rearmost edge of the body rather than off the last
  segment, so
  adding a segment neither shrinks it nor moves it — the newcomer has to grow far
  enough out to overtake the plate the tail was resting on before the tail starts
  travelling at all.
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
- **The ripple carries the legs, it does not stretch them.** The body's
  sideways wave is added to a leg's height as `side * HIP_OUT + wave`, not
  `side * (HIP_OUT + wave)`. The second form looks harmless and reads as a
  broken hip: it moves the near side's legs out with the wave and the far
  side's legs in against it, so one flank's hip goes from buried 102 units
  under its plate to 35 — long roots showing — while the other goes from 102 to
  163, which looks exactly like 102 because being *more* covered is invisible.
  It also makes the row of feet breathe in and out by twice the amplitude as
  the wave passes, when the only thing a sideways ripple should do to a track
  is slide it sideways.
- **A plate hangs by its middle, not its ridge.** The chevrons are cut from
  the sheet with their fold above the middle of the cut, so a plate hung on
  the fold hangs deeper on one flank than the other — 185 units of plate above
  the spine against 158 below it — and a hip planted between the two is tucked
  under the body on one side and left hanging in the open on the other. Hung
  by the middle of the cut instead (`SPINE_ANCHOR`'s `y`, its `x` still the
  apex) all three variants come out even, and `PLATE_EDGES` in `rig.rs` holds
  the art's own measurements so a test can hold the constants to them.
- **Both sides from one leg.** There is one leg in the rig, not two. It is
  solved with its foot out on the `+y` side of its hip, and a `side` of
  `-1` mirrors the result: the joint's `y`, the two angles, and the sign of
  the node's `y` scale all turn with it. `Transform::scale` is a plain
  diagonal matrix and the sprite pipeline culls no faces, so a negative
  `y` mirrors the clay cleanly — the row of legs on the other side of the
  body costs one float, and the two sides step half a cycle apart.
