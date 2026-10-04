//! The tools and the inventory dock: the four hand tools, their
//! cursor transforms and particle streams, the items panel's bays and
//! cells, and the held-items strip — the geometry and the `Demo`
//! methods that drive them. The tool half of input handling stays in
//! the frame loop; the swaps, tweens and emissions live here.

use crate::*;

/// One dock cell's leg of a tool swap: the cell's tool rides an arch to
/// the other cell, its pose written in the held panel's local space.
#[derive(Clone, Copy)]
pub(crate) struct CellFlight {
    /// The cell's rest spot, where the flight starts — or, for a swap
    /// pressed mid-flight, the cell's spot at the press.
    pub(crate) from: [f32; 2],
    /// The other cell's rest spot, where it lands.
    pub(crate) to: [f32; 2],
    /// The arch's height at its middle, in panel pixels: positive rises
    /// above the cells' line, negative dips below. Half the distance
    /// between the cells, by the swap's construction.
    pub(crate) bump: f32,
    /// Seconds flown so far.
    pub(crate) t: f32,
    /// The flight's total time, in seconds.
    pub(crate) dur: f32,
}

/// One swap flight's time, in seconds.
pub(crate) const SWAP_FLIGHT_TIME: f32 = 0.35;

/// The flying cell's tool spot at flight fraction `u`: the chord is
/// walked with a smoothstep — eased out of one cell, eased into the
/// other — while a parabola bends it off the chord, [CellFlight::bump]
/// high at `u` = half and back on the line at both ends.
pub(crate) fn cell_flight_pos(fly: &CellFlight, u: f32) -> [f32; 2] {
    let e = u * u * (3.0 - 2.0 * u);
    [
        fly.from[0] + (fly.to[0] - fly.from[0]) * e,
        fly.from[1] + (fly.to[1] - fly.from[1]) * e + fly.bump * (4.0 * u * (1.0 - u)),
    ]
}

/// `water_can_outline.png`'s texture size in pixels: the can's content,
/// cropped to the image.
pub(crate) const CAN_IMAGE: [f32; 2] = [333.0, 251.0];

/// The can's visible content in the image's own pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows to the right, `y` grows down. The
/// crop fills the image, so the content box is the whole texture.
pub(crate) const CAN_BOX: [[f32; 2]; 2] = [
    [0.0, 0.0],     // content upper-left
    [333.0, 251.0], // content lower-right
];

/// The rendered can's width in pixels; its height follows the content's
/// 333:251 aspect ratio.
pub(crate) const CAN_SIZE: f32 = 200.0;

/// Scales the whole texture so the can's content is `CAN_SIZE` wide.
pub(crate) const CAN_SCALE: f32 = CAN_SIZE / (CAN_BOX[1][0] - CAN_BOX[0][0]);

/// The can's rotated pose: a quarter turn counter-clockwise, applied while
/// the left mouse button is held down.
pub(crate) const CAN_ANGLE: f32 = std::f32::consts::FRAC_PI_2;

/// Seconds for the can to travel between its two poses, on press and on
/// release alike.
pub(crate) const ROTATE_TIME: f32 = 0.5;

/// The can's content center in node-local space: the sprite is centered on
/// its node's origin and the scene's y axis points up, so the content
/// center's image pixels `(cx, cy)` convert to `(cx - w/2, h/2 - cy)` —
/// the y flip included.
pub(crate) const CAN_LOCAL: [f32; 2] = {
    let cx = (CAN_BOX[0][0] + CAN_BOX[1][0]) / 2.0;
    let cy = (CAN_BOX[0][1] + CAN_BOX[1][1]) / 2.0;
    [cx - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - cy]
};

/// The spout tip in `water_can_outline.png`'s pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows right, `y` grows down.
pub(crate) const SPOUT: [f32; 2] = [0.0, 25.0];

/// The spout tip in node-local space, with the same y flip as
/// `CAN_LOCAL`; the water is emitted from here, rotated with the can.
pub(crate) const SPOUT_LOCAL: [f32; 2] =
    [SPOUT[0] - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - SPOUT[1]];

/// The node transform that puts the can's content center exactly on
/// `(mx, my)` and rotates the can by `angle` radians around that center.
///
/// The content center sits `CAN_LOCAL` (scaled by `CAN_SCALE`) from the
/// node's origin in node space, so the transform shifts the center to the
/// origin, rotates, then shifts back to the pointer. At `angle = 0` the
/// rotation is the identity, so this is exactly the unrotated
/// pointer-follow position.
pub(crate) fn can_transform(mx: f32, my: f32, angle: f32) -> frost::Transform {
    frost::Transform::translate([-CAN_LOCAL[0] * CAN_SCALE, -CAN_LOCAL[1] * CAN_SCALE])
        .compose(&frost::Transform::rotate(angle))
        .compose(&frost::Transform::translate([mx, my]))
}

/// The drops' emission rate, in drops per second, while the can is
/// fully tilted.
pub(crate) const RATE: f32 = 120.0;

/// The drops' launch speed range, in pixels per second, along the spout
/// direction.
pub(crate) const SPEED: (f32, f32) = (140.0, 240.0);

/// The half-width of the launch cone, in radians around the spout
/// direction.
pub(crate) const SPREAD: f32 = 0.12;

/// The drops' lifetime range, in seconds.
pub(crate) const LIFE: (f32, f32) = (0.5, 1.2);

/// The drops' radius range, in pixels.
pub(crate) const SIZE: (f32, f32) = (1.5, 3.5);

/// The gravity, in pixels per second, per second, pulling the drops
/// down (y points up, so it is negative).
pub(crate) const GRAVITY: f32 = 420.0;

/// The can is treated as fully tilted — and pouring — once it is within
/// this many radians of `CAN_ANGLE`.
pub(crate) const TILT_EPS: f32 = 0.05;

/// The spout tip's world (user) position and the direction the water
/// leaves it, for the can at `(mx, my)` rotated `angle` radians.
///
/// The tip sits `SPOUT_LOCAL` (scaled by `CAN_SCALE`) from the content
/// center in node space; rotating that offset by `angle` — the same
/// rotation the can itself undergoes, about the pointer — gives its
/// world offset. The water leaves along the rotated offset's direction,
/// i.e. straight out of the spout: at the full tilt that is nearly
/// straight down.
pub(crate) fn spout(mx: f32, my: f32, angle: f32) -> ([f32; 2], [f32; 2]) {
    let (ex, ey) = (SPOUT_LOCAL[0] * CAN_SCALE, SPOUT_LOCAL[1] * CAN_SCALE);
    let (c, s) = (angle.cos(), angle.sin());
    let (ox, oy) = (c * ex - s * ey, s * ex + c * ey);
    let l = (ox * ox + oy * oy).sqrt();
    ([mx + ox, my + oy], [ox / l, oy / l])
}

/// `items.png`'s texture size in pixels: the cropped panel itself.
pub(crate) const ITEMS_SIZE: [f32; 2] = [389.0, 991.0];

/// The clearance the panel keeps from the window's top, bottom, and left
/// borders, in pixels.
pub(crate) const MARGIN: f32 = 20.0;

/// `held_items.png`'s texture size in pixels; the held-items panel is
/// drawn at this natural size, `MARGIN` clear of the window's right and
/// bottom borders.
pub(crate) const HELD_SIZE: [f32; 2] = [389.0, 200.0];

/// `sustainable_immortality.png`'s texture size in pixels; the corner
/// badge is drawn at `IMMORTALITY_SCALE` of this natural size, `MARGIN`
/// clear of the window's top and right borders.
pub(crate) const IMMORTALITY_SIZE: [f32; 2] = [536.0, 548.0];

/// The badge's scale relative to its natural size: half of it.
pub(crate) const IMMORTALITY_SCALE: f32 = 0.5;

/// The clearance a held cell keeps from the edges of its half of the
/// panel, in pixels: past the panel's frame, with room to spare.
pub(crate) const HELD_INSET: f32 = 20.0;

/// The center of held cell `i` — 0 the left cell, 1 the right cell — in
/// the held node's local space: the half-panel center, y-flipped about
/// the panel's center. Both cells sit on the panel's horizontal midline,
/// so the local y is 0.
pub(crate) fn held_local(i: usize) -> [f32; 2] {
    let x = HELD_SIZE[0] / 4.0;
    [if i == 0 { -x } else { x }, 0.0]
}

/// The uniform scale that fits a `size`-pixel sprite into one held cell —
/// the panel's half width by its full height, keeping `HELD_INSET` clear
/// of the cell's edges.
pub(crate) fn held_scale(size: [f32; 2]) -> f32 {
    let w = HELD_SIZE[0] / 2.0 - 2.0 * HELD_INSET;
    let h = HELD_SIZE[1] - 2.0 * HELD_INSET;
    (w / size[0]).min(h / size[1])
}

/// The shift that centres a tool's *drawing* on whatever its node is
/// anchored to, at scale `s`, in that node's scaled space: a sprite's
/// content box is not always at its canvas's centre — `Tweezers.png`'s jaws
/// sit 37 px right of the middle of the canvas the tool was exported onto —
/// so anchoring the canvas leaves the drawing hanging out of whatever it was
/// fitted to. The cans are exported to their content and need nothing; the
/// spade needs a nudge; the tweezers need the whole correction.
pub(crate) fn drawn_centring(tool: Tool, s: f32) -> [f32; 2] {
    let box_ = tool.drawn();
    let img = tool.image_size();
    [
        -s * ((box_[0][0] + box_[1][0]) / 2.0 - img[0] / 2.0),
        -s * (img[1] / 2.0 - (box_[0][1] + box_[1][1]) / 2.0),
    ]
}

/// Where a tool's node belongs in held cell `i`: the cell's center, shifted
/// by [`drawn_centring`] so the drawing — the thing the player sees — sits
/// inside the cell it was scaled to fit, not merely its canvas.
pub(crate) fn cell_rest(i: usize, tool: Tool) -> [f32; 2] {
    let c = drawn_centring(tool, held_scale(tool.drawn_size()));
    [held_local(i)[0] + c[0], held_local(i)[1] + c[1]]
}

/// The shelf's slots: the four bays `items.png` paints, 0 (top) to 3
/// (bottom). The bays are drawn at their own heights — the chalk shelf is
/// not an even grid — so every measure of a slot comes from the sprite.
pub(crate) const SLOTS: usize = 4;

/// One bay per painted shelf, measured off `items.png`'s pixels at the
/// centre of its opening: `[opening centre x, rest y]`, `(0, 0)` the panel's
/// upper-left corner and `y` growing down. The rest line is the front of the
/// lit top of the board the bay stands on — the line a tool's feet go on,
/// deliberately out on the board's surface rather than against the back
/// wall, so a resting tool covers a little board and reads as standing in
/// the shelf instead of floating above it.
///
/// The line is where the profile of the sprite's luminance at `x` 170..205
/// turns up into each board's bright chalk edge, at `y` 264, 482, 700 and
/// 911; the same profile falls back under each line at 273, 490, 708 and 926
/// (see [`BAY_CUTS`]). Both series are evenly spaced — 218, 218, 211 and
/// 218, 218, 217 — which is what says the fourth of each is the bottom
/// board's own edge and not the shelf's frame below it, the frame being the
/// brighter band still 14 rows lower, just above the panel's transparent
/// bottom. How tall a bay is follows from its rest line and [`BAY_CUTS`],
/// which is where its tool's head stops: four boards that evenly spaced leave
/// the four bays near enough the same height, as a shelf should.
pub(crate) const BAYS: [[f32; 2]; SLOTS] = [
    [187.0, 264.0],
    [187.0, 482.0],
    [187.0, 700.0],
    [187.0, 911.0],
];

/// The width of a bay's opening in image pixels: the dark back wall between
/// the shelf's left upright (through `x` 76) and the open, see-through side
/// that starts at `x` 298.
pub(crate) const BAY_WIDTH: f32 = 220.0;

/// The room a resting tool keeps from its bay's ceiling, and from its
/// jambs, in pixels: the chalk boards are ragged and the shelf is drawn in
/// perspective, so nothing is fitted flush. Vertically the inset is counted
/// once, overhead only — a tool's feet are set on the board's rest line by
/// [`slot_rest`] itself, not left floating a clear inset above it.
pub(crate) const BAY_INSET: f32 = 12.0;

/// The cuts that carve the panel into four click cells, in image pixels, `y`
/// down: the middle of the board each bay stands on — midway between that
/// bay's rest line and the bright lower edge of the same board — with the
/// panel's own top and bottom edges beyond the outer bays. The cells
/// therefore tile the panel edge to edge and each one covers its bay's
/// whole opening and its board's front: a press anywhere on the shelf
/// belongs to a bay, and never leaks out as a tool use on the grass behind
/// it. The cut above a bay is also the ceiling of what may be drawn in it,
/// see [`bay_ceiling`].
pub(crate) const BAY_CUTS: [f32; SLOTS + 1] = [0.0, 268.5, 486.0, 704.0, ITEMS_SIZE[1]];

/// The underside of the shelf's top frame, in image pixels: the one ceiling
/// on the shelf that is not a cut between two bays, because there is no bay
/// above the frame to click through. Read off the sprite, where the frame's
/// front face ends at `y` 46..50.
pub(crate) const FRAME_UNDER: f32 = 50.0;

/// The line a tool standing in bay `i` may not be drawn above: the cut that
/// divides its click cell from the bay above, or [`FRAME_UNDER`] at the top
/// of the shelf. Picture and click box are then the same box — every pixel
/// of a resting tool is a pixel that answers to its bay, and no tool reaches
/// up into the shelf above its own.
pub(crate) fn bay_ceiling(i: usize) -> f32 {
    BAY_CUTS[i].max(FRAME_UNDER)
}

/// The slot the resting watering can sits in: the bottom bay.
pub(crate) const CAN_SLOT: usize = 3;

/// The slot the resting spray can sits in.
pub(crate) const SPRAY_SLOT: usize = 2;

/// The slot the resting spade sits in.
pub(crate) const SPADE_SLOT: usize = 1;

/// The slot the resting tweezers sit in.
pub(crate) const TWEEZERS_SLOT: usize = 0;

/// The tools the items panel starts with, in slot order: one tool per
/// painted bay — the tweezers, the spade, the spray can, and the watering
/// can, top to bottom. The scene's seeded slot sprites and [`Demo::new`]
/// both come from here, so the shelf's picture and the demo's table can
/// never part company. A tool parked in a bay swaps with the one resting
/// there, so every bay is both a home and a hand-over point.
pub(crate) const fn seeded_slots() -> [Option<Tool>; SLOTS] {
    let mut slots = [None; SLOTS];
    slots[TWEEZERS_SLOT] = Some(Tool::Tweezers);
    slots[SPADE_SLOT] = Some(Tool::Spade);
    slots[SPRAY_SLOT] = Some(Tool::SprayCan);
    slots[CAN_SLOT] = Some(Tool::WaterCan);
    slots
}

/// [seeded_slots] as a value: the shelf's starting layout.
pub(crate) const SEEDED_SLOTS: [Option<Tool>; SLOTS] = seeded_slots();

/// The center of slot `i`'s click cell in the items node's local space: the
/// bay's opening centre in `x`, the middle of the cell's cut in `y`, both
/// y-flipped about the panel's center — so a child translated here lands in
/// the middle of the bay, and [`slot_hovered`] tests the same box.
pub(crate) fn slot_local(i: usize) -> [f32; 2] {
    let mid = (BAY_CUTS[i] + BAY_CUTS[i + 1]) / 2.0;
    [BAYS[i][0] - ITEMS_SIZE[0] / 2.0, ITEMS_SIZE[1] / 2.0 - mid]
}

/// A slot click cell's half extents in image pixels: the panel's full width
/// (the shelf's open right side belongs to its bay too) by the bay's own
/// cut.
pub(crate) fn slot_half(i: usize) -> [f32; 2] {
    [ITEMS_SIZE[0] / 2.0, (BAY_CUTS[i + 1] - BAY_CUTS[i]) / 2.0]
}

/// The largest uniform scale a `size`-pixel tool's drawn content fits at in
/// bay `i`: the width between its jambs less `BAY_INSET` on each side, and
/// the headroom from its rest line up to [`bay_ceiling`] less the inset
/// overhead.
pub(crate) fn bay_scale(i: usize, size: [f32; 2]) -> f32 {
    let [_, rest] = BAYS[i];
    let w = BAY_WIDTH - 2.0 * BAY_INSET;
    let h = rest - bay_ceiling(i) - BAY_INSET;
    (w / size[0]).min(h / size[1])
}

/// The scale a `tool` is drawn at on the shelf — the same in every bay.
/// Each bay can hold a different height, so the scale that lets a tool stand
/// anywhere without changing size is the tightest of them: fitting the
/// shelf's shallowest bay, and the width of its jambs, is what a tool's size
/// means here. A tool is therefore never resized by where it is parked, and
/// never drawn larger than the space it is standing in.
pub(crate) fn tool_scale(tool: Tool) -> f32 {
    let size = tool.drawn_size();
    (0..SLOTS)
        .map(|i| bay_scale(i, size))
        .fold(f32::MAX, f32::min)
}

/// Where a resting `tool` sits in bay `i`, at the one scale [`tool_scale`]
/// gives it: the tool's drawn content centred on the bay's opening, standing
/// on its board's rest line, not floated at the click cell's middle. The
/// content box is off-centre in most textures, so the node — the texture's
/// center — takes the correction that puts the drawing where the shelf says
/// it should be. Only the bay's lines come from `i`; the size does not.
pub(crate) fn slot_rest(i: usize, tool: Tool) -> ([f32; 2], f32) {
    let dy = tool.drawn_size()[1];
    let s = tool_scale(tool);
    let c = drawn_centring(tool, s);
    let [opening, rest] = BAYS[i];
    (
        [
            opening - ITEMS_SIZE[0] / 2.0 + c[0],
            // The centring puts the content's centre on the node; the feet
            // belong on the board, so the node rides half the drawing above.
            ITEMS_SIZE[1] / 2.0 - rest + c[1] + s * dy / 2.0,
        ],
        s,
    )
}

/// Whether the user-space point `p` is inside slot `i`, for the panel node
/// `items`: the slot's click cell in the panel's local space — the full
/// panel width by that bay's own cut, centered on `slot_local(i)` — mapped
/// through the panel's world transform, the same scale-then-transform
/// composition the renderer draws it with.
pub(crate) fn slot_hovered(items: &frost::SceneNode, i: usize, p: [f32; 2]) -> bool {
    let world = frost::Transform::scale(items.scale).compose(&items.transform);
    let [cx, cy] = world.apply(slot_local(i));
    let [hw, hh] = slot_half(i);
    (p[0] - cx).abs() <= hw * items.scale[0] && (p[1] - cy).abs() <= hh * items.scale[1]
}

/// `Spray1.png` and `Spray2.png`'s texture size in pixels: both frames
/// are the same size and are drawn at their natural size, unscaled.
pub(crate) const SPRAY_IMAGE: [f32; 2] = [136.0, 276.0];

/// The spray can's pivot in the `Spray{1,2}.png` pixel space: the point
/// that lands on the cursor, and the point about which the can rotates
/// during a burst. `(0, 0)` is the upper-left corner, `x` grows right,
/// `y` grows down.
pub(crate) const SPRAY_PIVOT: [f32; 2] = [98.0, 60.0];

/// The pivot in node-local space, with the same y flip as
/// `SPOUT_LOCAL`; the spray can's node transform keeps it on the cursor.
pub(crate) const SPRAY_PIVOT_LOCAL: [f32; 2] = [
    SPRAY_PIVOT[0] - SPRAY_IMAGE[0] / 2.0,
    SPRAY_IMAGE[1] / 2.0 - SPRAY_PIVOT[1],
];

/// The nozzle tip in the `Spray{1,2}.png` pixel space: the point where
/// the green spray is emitted, rotated with the can.
pub(crate) const SPRAY_NOZZLE: [f32; 2] = [21.0, 13.0];

/// The nozzle tip in node-local space, with the same y flip as
/// `SPRAY_PIVOT_LOCAL`.
pub(crate) const SPRAY_NOZZLE_LOCAL: [f32; 2] = [
    SPRAY_NOZZLE[0] - SPRAY_IMAGE[0] / 2.0,
    SPRAY_IMAGE[1] / 2.0 - SPRAY_NOZZLE[1],
];

/// The burst's tilt, in radians: 45 degrees clockwise, negative because
/// the scene's y axis points up and counter-clockwise is positive.
pub(crate) const BURST_ANGLE: f32 = -std::f32::consts::FRAC_PI_4;

/// Seconds for a whole burst: `BURST_HALF` of clockwise tilting and
/// `BURST_HALF` of the return.
pub(crate) const BURST_TIME: f32 = 0.25;

pub(crate) const BURST_HALF: f32 = BURST_TIME / 2.0;

/// The spray's emission rate, in particles per second, while the burst is
/// tilting.
pub(crate) const SPRAY_RATE: f32 = 300.0;

/// The spray's launch speed range, in pixels per second, along the nozzle
/// direction.
pub(crate) const SPRAY_SPEED: (f32, f32) = (160.0, 380.0);

/// The half-width of the launch cone, in radians around the nozzle
/// direction.
pub(crate) const SPRAY_SPREAD: f32 = 0.4;

/// The spray particles' lifetime range, in seconds.
pub(crate) const SPRAY_LIFE: (f32, f32) = (0.35, 0.8);

/// The spray particles' radius range, in pixels.
pub(crate) const SPRAY_SIZE: (f32, f32) = (1.0, 2.5);

/// The gravity, in pixels per second, per second, pulling the spray down:
/// lighter than `GRAVITY`, because mist falls slower than drops.
pub(crate) const SPRAY_GRAVITY: f32 = 120.0;

/// The spray's color; the batched draw scales each drop's alpha by its
/// remaining life fraction.
pub(crate) const SPRAY: frost::Color = frost::Color {
    r: 0.45,
    g: 0.9,
    b: 0.4,
    a: 1.0,
};

/// The node transform that puts the spray can's pivot exactly on `(mx, my)`
/// and rotates the can by `angle` radians around that pivot.
///
/// The pivot sits `SPRAY_PIVOT_LOCAL` from the node's origin in node
/// space, so the transform shifts the pivot to the origin, rotates, then
/// shifts back to the pointer. At `angle = 0` the rotation is the
/// identity, so this is exactly the unrotated pointer-follow position.
pub(crate) fn spray_transform(mx: f32, my: f32, angle: f32) -> frost::Transform {
    frost::Transform::translate([-SPRAY_PIVOT_LOCAL[0], -SPRAY_PIVOT_LOCAL[1]])
        .compose(&frost::Transform::rotate(angle))
        .compose(&frost::Transform::translate([mx, my]))
}

/// The nozzle tip's world (user) position and the direction the spray
/// leaves it, for the can at `(mx, my)` rotated `angle` radians.
///
/// The tip sits `SPRAY_NOZZLE_LOCAL - SPRAY_PIVOT_LOCAL` from the pivot in
/// node space; rotating that offset by `angle` — the same rotation the can
/// itself undergoes, about the pivot, which sits on the cursor — gives its
/// world offset. The spray leaves along the rotated offset's direction,
/// i.e. straight out of the nozzle: at the full tilt that is nearly
/// straight up.
pub(crate) fn nozzle(mx: f32, my: f32, angle: f32) -> ([f32; 2], [f32; 2]) {
    let (ex, ey) = (
        SPRAY_NOZZLE_LOCAL[0] - SPRAY_PIVOT_LOCAL[0],
        SPRAY_NOZZLE_LOCAL[1] - SPRAY_PIVOT_LOCAL[1],
    );
    let (c, s) = (angle.cos(), angle.sin());
    let (ox, oy) = (c * ex - s * ey, s * ex + c * ey);
    let l = (ox * ox + oy * oy).sqrt();
    ([mx + ox, my + oy], [ox / l, oy / l])
}

/// `spade.png`'s texture size in pixels: the chalk spade sits inside a
/// 337x256 canvas, with a transparent margin all round it.
pub(crate) const SPADE_IMAGE: [f32; 2] = [337.0, 256.0];

/// The spade's visible content in the image's own pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows to the right, `y` grows down. Measured
/// off the texture's alpha: the handle's knob at the canvas's upper right,
/// the blade's point at its lower left.
pub(crate) const SPADE_BOX: [[f32; 2]; 2] = [
    [49.0, 23.0],   // content upper-left
    [293.0, 237.0], // content lower-right
];

/// The rendered spade's width in pixels; its height follows the content's
/// 244:214 aspect ratio.
pub(crate) const SPADE_SIZE: f32 = 210.0;

/// Scales the whole texture so the spade's content is `SPADE_SIZE` wide.
pub(crate) const SPADE_SCALE: f32 = SPADE_SIZE / (SPADE_BOX[1][0] - SPADE_BOX[0][0]);

/// The grip in `spade.png`'s pixel space: the middle of the wooden handle,
/// where the hand holds the spade. The dig swings the whole tool about
/// this point, so the cursor rests here.
pub(crate) const SPADE_GRIP: [f32; 2] = [237.0, 78.0];

/// The grip in node-local space, with the same y flip as `CAN_LOCAL`; the
/// spade's node transform keeps it on the cursor.
pub(crate) const SPADE_GRIP_LOCAL: [f32; 2] = frost::Transform::anchor(SPADE_GRIP, SPADE_IMAGE);

/// The blade's point in `spade.png`'s pixel space: the tip of the trowel,
/// diagonally opposite the grip — the part that goes into the soil.
pub(crate) const SPADE_TIP: [f32; 2] = [49.0, 209.0];

/// The blade's point in node-local space, with the same y flip as
/// `CAN_LOCAL`.
pub(crate) const SPADE_TIP_LOCAL: [f32; 2] = frost::Transform::anchor(SPADE_TIP, SPADE_IMAGE);

/// The spade's drawn tilt, in radians: the direction from the grip to the
/// blade's point, read off the sprite's own pixels. The point lies 188 px
/// left and 131 px below the grip, so the spade as drawn aims 34.9° below
/// the horizontal, point to the lower left. That is the direction the dig
/// pushes along: the tool goes where its own point already points, so the
/// stroke needs no angle of its own.
pub(crate) fn spade_blade() -> f32 {
    (SPADE_TIP_LOCAL[1] - SPADE_GRIP_LOCAL[1]).atan2(SPADE_TIP_LOCAL[0] - SPADE_GRIP_LOCAL[0])
}

/// The stroke's three beats, in seconds: the spade is shoved forward along
/// its own line in `DIG_PUSH`, stays buried for `DIG_HOLD`, then travels
/// home along an arch in `DIG_ARC`.
pub(crate) const DIG_PUSH: f32 = 0.16;

pub(crate) const DIG_HOLD: f32 = 0.30;

pub(crate) const DIG_ARC: f32 = 0.34;

/// Seconds for one dig stroke: the three beats, so they can never stop
/// adding up to the whole.
pub(crate) const DIG_TIME: f32 = DIG_PUSH + DIG_HOLD + DIG_ARC;

/// How far the push goes, in pixels measured along the blade's own line,
/// point first: the grip leaves the cursor by this much and no further.
pub(crate) const DIG_PUSH_DIST: f32 = 56.0;

/// The arch's crown, in pixels from the cursor — `x` right, `y` up. The way
/// home is not back along the push: the spade lifts out and sweeps up and to
/// the left of the cursor, over the ground it just broke, and settles onto
/// its anchor from there. The arch is a quadratic Bézier from the buried
/// spot to the cursor through the control point [`dig_arch_control`] puts
/// beyond this crown, so the path passes through the crown itself.
pub(crate) const DIG_ARC_CROWN: [f32; 2] = [-74.0, 46.0];

/// The Bézier control point that lifts the arch over [`DIG_ARC_CROWN`] from
/// the buried spot `[0, 0]` back to the cursor: a quadratic reads its crown
/// at the halfway mark as a quarter of the start plus half the control, so
/// twice the crown less half the start is the control that lands it there.
pub(crate) fn dig_arch_control(push: [f32; 2]) -> [f32; 2] {
    [
        2.0 * DIG_ARC_CROWN[0] - 0.5 * push[0],
        2.0 * DIG_ARC_CROWN[1] - 0.5 * push[1],
    ]
}

/// The symmetric ease the stroke's beats are walked with: slow to leave,
/// slow to arrive.
pub(crate) fn smooth(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

/// The dig's offset at fraction `u` of the stroke: how far the grip — and so
/// the whole spade, which does not turn for a dig — has left the cursor.
/// Three beats, one path: shoved point-first down its own drawn line, eased
/// out of the hand onto the soil; held there, buried, for `DIG_HOLD`; then
/// arching up and to the left and home to the cursor, arriving exactly where
/// it started. `u` = 0 and `u` = 1 are both the drawn pose on the anchor.
pub(crate) fn dig_offset(u: f32) -> [f32; 2] {
    let t = u.clamp(0.0, 1.0) * DIG_TIME;
    // The line the spade travels along: its own, point first, down-left.
    let push = [
        DIG_PUSH_DIST * spade_blade().cos(),
        DIG_PUSH_DIST * spade_blade().sin(),
    ];
    if t <= DIG_PUSH {
        let e = smooth(t / DIG_PUSH);
        [push[0] * e, push[1] * e]
    } else if t <= DIG_PUSH + DIG_HOLD {
        push
    } else {
        // The arch home. A quadratic Bézier between the buried spot and the
        // cursor, walked with the same ease the push is eased with, so the
        // stroke leaves the soil and lands on the anchor gently.
        let e = smooth((t - DIG_PUSH - DIG_HOLD) / DIG_ARC);
        let c = dig_arch_control(push);
        let (a, b) = (1.0 - e, e);
        [
            a * a * push[0] + 2.0 * a * b * c[0],
            a * a * push[1] + 2.0 * a * b * c[1],
        ]
    }
}

/// The node transform that puts the spade's grip on `(mx, my)` plus the
/// dig's offset `off`.
///
/// The grip sits `SPADE_GRIP_LOCAL` (scaled by `SPADE_SCALE`) from the
/// node's origin in node space, so the transform shifts the grip to the
/// origin and then to the pointer plus the dig's offset `off` — the spade
/// never turns, it is carried. With no offset this is exactly the
/// pointer-follow position, the spade riding the cursor as drawn.
pub(crate) fn spade_transform(mx: f32, my: f32, off: [f32; 2]) -> frost::Transform {
    frost::Transform::translate([
        -SPADE_GRIP_LOCAL[0] * SPADE_SCALE,
        -SPADE_GRIP_LOCAL[1] * SPADE_SCALE,
    ])
    .compose(&frost::Transform::translate([mx + off[0], my + off[1]]))
}

/// `Tweezers.png`'s texture size in pixels: the same 337x256 canvas as the
/// spade, the chalk drawing inside it.
pub(crate) const TWEEZERS_IMAGE: [f32; 2] = [337.0, 256.0];

/// The tweezers' visible content in the image's own pixel space, with the
/// same corners-as-pixels convention as `SPADE_BOX`: the joined end at the
/// canvas's upper right, the two jaws' points at its lower left.
pub(crate) const TWEEZERS_BOX: [[f32; 2]; 2] = [
    [81.0, 49.0],   // content upper-left
    [330.0, 218.0], // content lower-right
];

/// The rendered tweezers' width in pixels; their height follows the
/// content's 249:169 aspect ratio.
pub(crate) const TWEEZERS_SIZE: f32 = 190.0;

/// Scales the whole texture so the tweezers' content is `TWEEZERS_SIZE`
/// wide.
pub(crate) const TWEEZERS_SCALE: f32 = TWEEZERS_SIZE / (TWEEZERS_BOX[1][0] - TWEEZERS_BOX[0][0]);

/// The jaws' points in `Tweezers.png`'s pixel space: the working end,
/// between the two prongs' tips — where a louse will sit once the pinch
/// frames exist. The cursor rests here.
pub(crate) const TWEEZERS_TIP: [f32; 2] = [95.0, 200.0];

/// The jaws' points in node-local space, with the same y flip as
/// `CAN_LOCAL`.
pub(crate) const TWEEZERS_TIP_LOCAL: [f32; 2] =
    frost::Transform::anchor(TWEEZERS_TIP, TWEEZERS_IMAGE);

/// The node transform that puts the tweezers' jaws' points on `(mx, my)`:
/// the same shift-and-shift shape as `can_transform`, minus the rotation —
/// the tweezers ride level until there is a pinch to animate.
pub(crate) fn tweezers_transform(mx: f32, my: f32) -> frost::Transform {
    frost::Transform::translate([
        mx - TWEEZERS_TIP_LOCAL[0] * TWEEZERS_SCALE,
        my - TWEEZERS_TIP_LOCAL[1] * TWEEZERS_SCALE,
    ])
}

/// The four cursor tools. The mouse holds one active tool, drawn as the
/// cursor, and one stored tool, mirrored in the held-items panel: the
/// right-button switch swaps the two, and a left click on a slot swaps the
/// active tool with the slot's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Tool {
    /// The watering can: hold the left button to tilt it and pour water.
    WaterCan,
    /// The spray can: a fresh left-button press triggers a green burst.
    SprayCan,
    /// The garden spade: a fresh left-button press digs one stroke.
    Spade,
    /// The tweezers: carried and exchanged like the rest, but still
    /// inert — picking needs the pinch frames this one sprite has no pair
    /// for.
    Tweezers,
}

impl Tool {
    /// The tool's drawn content box in its texture's pixels, `[upper-left,
    /// lower-right]`, measured off the sprite's alpha. The shelf's slots
    /// and the held cells fit this rather than the whole texture, so a
    /// drawing with a wide transparent margin around it — the spade and
    /// the tweezers — still fills its cell.
    pub(crate) fn drawn(self) -> [[f32; 2]; 2] {
        match self {
            // Both cans are cropped to their content: the texture is the
            // drawing.
            Tool::WaterCan => [[0.0, 0.0], [CAN_IMAGE[0], CAN_IMAGE[1]]],
            Tool::SprayCan => [[0.0, 0.0], [SPRAY_IMAGE[0], SPRAY_IMAGE[1]]],
            Tool::Spade => SPADE_BOX,
            Tool::Tweezers => TWEEZERS_BOX,
        }
    }

    /// The tool's drawn content size in its texture's pixels.
    pub(crate) fn drawn_size(self) -> [f32; 2] {
        let box_ = self.drawn();
        [box_[1][0] - box_[0][0], box_[1][1] - box_[0][1]]
    }

    /// The tool's texture size in pixels: what its sprite node spans, as
    /// opposed to [`Tool::drawn_size`], what the drawing spans inside that.
    /// The two come apart by the transparent margin the art ships with —
    /// a third of the spade's texture is nothing but air around the tool.
    pub(crate) fn image_size(self) -> [f32; 2] {
        match self {
            Tool::WaterCan => CAN_IMAGE,
            Tool::SprayCan => SPRAY_IMAGE,
            Tool::Spade => SPADE_IMAGE,
            Tool::Tweezers => TWEEZERS_IMAGE,
        }
    }

    /// The scale the tool's sprite rides at as the cursor: the cans scaled
    /// to their rendered widths, the spray can and its two frames at their
    /// natural size.
    pub(crate) fn cursor_scale(self) -> f32 {
        match self {
            Tool::WaterCan => CAN_SCALE,
            Tool::SprayCan => 1.0,
            Tool::Spade => SPADE_SCALE,
            Tool::Tweezers => TWEEZERS_SCALE,
        }
    }

    /// The tool's at-rest sprite out of the loaded `assets`: the at-rest
    /// frame for the tools with more than one.
    pub(crate) fn sprite(self, assets: &Assets) -> frost::Shape {
        match self {
            Tool::WaterCan => assets.can.clone(),
            Tool::SprayCan => assets.spray1.clone(),
            Tool::Spade => assets.spade.clone(),
            Tool::Tweezers => assets.tweezers.clone(),
        }
    }
}

impl Demo {
    /// Ticks the active tool's live pose every frame so an ongoing
    /// tilt, return, burst, or dig keeps moving; no tool held is a no-op.
    /// The watering can pours at the full tilt, the spray can runs its
    /// burst to `BURST_TIME`, the spade runs its stroke to `DIG_TIME`, the
    /// tweezers have nothing to run yet, the pour sound starts and stops
    /// with the tilt, and the tool node takes the live transform.
    pub(crate) fn tick_tool(&mut self, ctx: &mut frost::Context, dt: f32) {
        let [mx, my] = self.mouse;

        // The active tool's live pose, ticked every frame so an ongoing
        // tilt, return, or burst keeps moving; no tool held is a no-op.
        match self.active {
            // The watering can pours at the full tilt.
            Some(Tool::WaterCan) => {
                self.angle = self.rotation.tick(dt);

                // Water pours out of the spout once the can is fully
                // tilted: one drop per whole unit of the accumulator,
                // launched from the tip (rotated with the can) along the
                // spout direction.
                if self.angle >= CAN_ANGLE - TILT_EPS {
                    let ([sx, sy], [dx, dy]) = spout(mx, my, self.angle);
                    let base = dy.atan2(dx);
                    self.acc += RATE * dt;
                    while self.acc >= 1.0 {
                        self.acc -= 1.0;
                        let a = base + self.rng.in_range(-SPREAD, SPREAD);
                        let life = self.rng.in_range(LIFE.0, LIFE.1);
                        self.water.spawn(frost::Particle {
                            pos: [sx, sy],
                            vel: [
                                a.cos() * self.rng.in_range(SPEED.0, SPEED.1),
                                a.sin() * self.rng.in_range(SPEED.0, SPEED.1),
                            ],
                            life,
                            max_life: life,
                            size: self.rng.in_range(SIZE.0, SIZE.1),
                            angle: 0.0,
                            color: frost::Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            },
                        });
                    }
                }
            }
            // The spray can: the burst, once a press started it, runs to
            // `BURST_TIME`; nothing to do while no burst is in flight.
            Some(Tool::SprayCan) => {
                if let Some(t) = self.burst {
                    let nt = t + dt;
                    if nt >= BURST_TIME {
                        // The burst is over: the can stands upright again,
                        // showing the at-rest frame.
                        self.burst = None;
                        self.angle = 0.0;
                        self.set_spray_frame(ctx, false);
                    } else {
                        self.burst = Some(nt);
                        self.angle = self.rotation.tick(dt);
                        // Emission is limited to the first half — the
                        // clockwise tilting: one particle per whole unit of
                        // the accumulator, launched from the nozzle
                        // (rotated with the can) along the nozzle
                        // direction.
                        if nt < BURST_HALF {
                            let ([nx, ny], [dx, dy]) = nozzle(mx, my, self.angle);
                            let base = dy.atan2(dx);
                            self.acc += SPRAY_RATE * dt;
                            while self.acc >= 1.0 {
                                self.acc -= 1.0;
                                let a = base + self.rng.in_range(-SPRAY_SPREAD, SPRAY_SPREAD);
                                let life = self.rng.in_range(SPRAY_LIFE.0, SPRAY_LIFE.1);
                                self.spray.spawn(frost::Particle {
                                    pos: [nx, ny],
                                    vel: [
                                        a.cos() * self.rng.in_range(SPRAY_SPEED.0, SPRAY_SPEED.1),
                                        a.sin() * self.rng.in_range(SPRAY_SPEED.0, SPRAY_SPEED.1),
                                    ],
                                    life,
                                    max_life: life,
                                    size: self.rng.in_range(SPRAY_SIZE.0, SPRAY_SIZE.1),
                                    angle: 0.0,
                                    color: frost::Color {
                                        r: 1.0,
                                        g: 1.0,
                                        b: 1.0,
                                        a: 1.0,
                                    },
                                });
                            }
                        }
                    }
                }
            }
            // No tool held: nothing to tilt, burst, or emit.
            None => {}
            // The spade: the stroke, once a press started it, runs to
            // `DIG_TIME`; its whole pose is the grip's offset from the
            // cursor, [`dig_offset`], and nothing is emitted.
            Some(Tool::Spade) => {
                if let Some(t) = self.dig {
                    let nt = t + dt;
                    if nt >= DIG_TIME {
                        // The stroke is over: the spade sits in the pose it
                        // is drawn in again, on its anchor.
                        self.dig = None;
                    } else {
                        self.dig = Some(nt);
                    }
                }
            }
            // The tweezers: carried, but with no action to tick yet.
            Some(Tool::Tweezers) => {}
        }

        // The pour sound follows the spout: it starts, looping, the frame
        // water actually pours out of the can — the watering can active
        // and fully tilted, the same condition the drops spawn on — and
        // stops the frame pouring ends, the can turning back upright or
        // the tool switching away from it.
        let pouring = self.active == Some(Tool::WaterCan) && self.angle >= CAN_ANGLE - TILT_EPS;
        if pouring != self.pouring {
            self.pouring = pouring;
            if pouring {
                self.sounds.device.play_loop(&self.sounds.pour);
            } else {
                self.sounds.device.stop_loop();
            }
        }

        // A fresh transform is only needed while the node shows a tool.
        if let Some(tool) = self.active {
            let tool_node = &mut ctx.scene().root.children[CHILD_TOOL];
            tool_node.transform = match tool {
                Tool::WaterCan => can_transform(mx, my, self.angle),
                Tool::SprayCan => spray_transform(mx, my, self.angle),
                Tool::Spade => spade_transform(mx, my, self.dig_offset()),
                Tool::Tweezers => tweezers_transform(mx, my),
            };
        }
    }

    /// The dig stroke's grip offset from the cursor, in user pixels:
    /// [`dig_offset`] at the stroke's live time, and nothing at all while no
    /// stroke is running.
    pub(crate) fn dig_offset(&self) -> [f32; 2] {
        self.dig.map_or([0.0, 0.0], |t| dig_offset(t / DIG_TIME))
    }

    /// Advances the particles even while not emitting, so an ongoing
    /// stream keeps falling until it dies out.
    pub(crate) fn update_particles(&mut self, dt: f32) {
        self.water.update(dt, [0.0, -GRAVITY]);
        self.spray.update(dt, [0.0, -SPRAY_GRAVITY]);
    }

    /// Makes `tool` the active tool — `None` for no tool: it resets the
    /// can to its upright, at-rest pose (angle, burst, dig, rotation tween,
    /// and frame) and puts the tool's shape and scale on the node, or
    /// clears it, so no tool inherits another's tilt, burst, stroke, or
    /// sprite.
    pub(crate) fn set_active(&mut self, ctx: &mut frost::Context, tool: Option<Tool>) {
        self.active = tool;
        self.angle = 0.0;
        self.burst = None;
        self.dig = None;
        self.rotation = frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once);
        self.showing_spray2 = false;
        let tool_node = &mut ctx.scene().root.children[CHILD_TOOL];
        match tool {
            Some(tool) => {
                tool_node.shape = Some(self.tool_shape(tool));
                let s = tool.cursor_scale();
                tool_node.scale = [s, s];
            }
            None => {
                tool_node.shape = None;
            }
        }
        // Mirror both tools into the held-items panel: the active one in
        // the left cell, the stored one in the right. Every tool change —
        // the right-button switch and the slot swaps alike — goes through
        // this method, so this single sync keeps the panel current.
        self.sync_held(ctx);
    }

    /// The tool's at-rest shape, cloned out of the demo's own copies: the
    /// at-rest frame for the tools with more than one, and a cheap `Arc`
    /// clone of the pixel buffer.
    pub(crate) fn tool_shape(&self, tool: Tool) -> frost::Shape {
        match tool {
            Tool::WaterCan => self.can.clone(),
            Tool::SprayCan => self.spray1.clone(),
            Tool::Spade => self.spade.clone(),
            Tool::Tweezers => self.tweezers.clone(),
        }
    }

    /// Swaps the active tool with whatever rests in slot `slot`: the
    /// slot's tool becomes the active one — or nothing, if the slot is
    /// empty, in which case the mouse shows no sprite, still holding the
    /// stored tool, if any — and the active tool moves into the slot.
    pub(crate) fn swap_with_slot(&mut self, ctx: &mut frost::Context, slot: usize) {
        let incoming = self.active;
        let outgoing = self.slots[slot];
        self.slots[slot] = incoming;
        self.set_active(ctx, outgoing);
        self.slot_set(
            slot,
            &mut ctx.scene().root.children[CHILD_ITEMS].children[slot],
            incoming,
        );
    }

    /// Rests `tool` — or nothing — in bay `slot`'s node: the tool's shape at
    /// its bay's fit scale, standing on that bay's rest line, or no shape at
    /// all.
    pub(crate) fn slot_set(
        &mut self,
        slot: usize,
        node: &mut frost::SceneNode,
        tool: Option<Tool>,
    ) {
        match tool {
            Some(tool) => {
                let (t, s) = slot_rest(slot, tool);
                node.shape = Some(self.tool_shape(tool));
                node.scale = [s, s];
                node.transform = frost::Transform::translate(t);
            }
            None => node.shape = None,
        }
    }

    /// Shows the pressed spray frame (`spray2`) if `on` and the at-rest
    /// frame (`spray1`) otherwise, but only when the tool node currently
    /// shows the other one, so the swap — a cheap `Arc` clone — happens at
    /// most once per change.
    pub(crate) fn set_spray_frame(&mut self, ctx: &mut frost::Context, on: bool) {
        if self.showing_spray2 == on {
            return;
        }
        self.showing_spray2 = on;
        ctx.scene().root.children[CHILD_TOOL].shape = Some(if on {
            self.spray2.clone()
        } else {
            self.spray1.clone()
        });
    }

    /// Arcs the swap flights, one per dock cell, along their mirrored
    /// arches; on landing the cells snap to their rest spots and show
    /// the swapped tools. With no flight in the air the cells simply
    /// hold their rest spots — a reload or restart mid-flight can never
    /// strand a tool off its cell.
    pub(crate) fn step_cell_flight(&mut self, ctx: &mut frost::Context, dt: f32) {
        match &mut self.cell_fly {
            Some(flies) => {
                let mut done = true;
                for (fly, cell) in flies
                    .iter_mut()
                    .zip(ctx.scene().root.children[CHILD_HELD].children.iter_mut())
                {
                    fly.t = (fly.t + dt).min(fly.dur);
                    done &= fly.t >= fly.dur;
                    let p = cell_flight_pos(fly, fly.t / fly.dur);
                    cell.transform = frost::Transform::translate(p);
                }
                if done {
                    self.cell_fly = None;
                    // Landing is `sync_held`'s business: it writes both the
                    // tools and the spots they land on, each centred on the
                    // drawing it carries.
                    self.sync_held(ctx);
                }
            }
            None => {
                // No flight in the air: every cell holds its own rest spot
                // — its cell's center, nudged to centre the drawing it
                // carries, the same spot `sync_held` writes.
                let tools = [self.active, self.held];
                let held = &mut ctx.scene().root.children[CHILD_HELD];
                for (i, cell) in held.children.iter_mut().take(2).enumerate() {
                    cell.transform = frost::Transform::translate(match tools[i] {
                        Some(tool) => cell_rest(i, tool),
                        None => held_local(i),
                    });
                }
            }
        }
    }

    /// Mirrors the mouse's two tools into the held-items panel: the active
    /// tool in the left cell, the stored one in the right, each at its
    /// at-rest frame in the held-fit scale — or an empty cell for `None`.
    pub(crate) fn sync_held(&mut self, ctx: &mut frost::Context) {
        // While the swap flights are in the air each cell rides with —
        // and shows — the tool leaving it; the swap only reads true in
        // the cells once both have landed.
        let (left, right) = if self.cell_fly.is_some() {
            (self.held, self.active)
        } else {
            (self.active, self.held)
        };
        let held = &mut ctx.scene().root.children[CHILD_HELD];
        self.cell_set(0, &mut held.children[0], left);
        self.cell_set(1, &mut held.children[1], right);
    }

    /// Puts `tool` — or nothing — in a held cell's node: the tool's shape
    /// at the held-fit scale, always the at-rest frame, or no shape at
    /// all.
    pub(crate) fn cell_set(
        &mut self,
        cell: usize,
        node: &mut frost::SceneNode,
        tool: Option<Tool>,
    ) {
        match tool {
            Some(tool) => {
                node.shape = Some(self.tool_shape(tool));
                let s = held_scale(tool.drawn_size());
                node.scale = [s, s];
                node.transform = frost::Transform::translate(cell_rest(cell, tool));
            }
            None => {
                node.shape = None;
                node.transform = frost::Transform::translate(held_local(cell));
            }
        }
    }
}
