//! The open penumbra, tested against geometry rather than against itself.
//!
//! Every other shadow test in this tree runs `penumbra: 0.0`, which takes the
//! point ray in `lighting.wgsl` and never touches the disk of shadow rays the
//! soft edge is made of — the most expensive path in the engine and the least
//! covered one. The obvious test of it is a CPU twin: mirror the tap offsets,
//! the rotation and the segment clip, and compare pixel by pixel. That was
//! tried once and disagreed with the GPU on 692 of 4096 pixels under an
//! *unmodified* shader, which proved the twin was wrong and left the
//! disagreement unexplained (docs/benchmarking.md §20). A test that needs a
//! second implementation to be right before it can say anything is not much of
//! a test.
//!
//! So this file asserts properties instead, ones the geometry forces however
//! the disk is sampled:
//!
//! * A pixel the caster covers from every point of the light is dark; one it
//!   covers from no point is fully lit.
//! * The **outer silhouette** of the shadow — the first ray blocked, on each
//!   flank — must be centred on the axis through the light's centre and the
//!   caster's. Whatever rays a rung asks for, they are placed about the light's
//!   own centre, so the outline of the shadow they cast is symmetric about that
//!   line. A tap set whose samples are not centred on the light shifts it, and
//!   that is the defect this file exists to catch: in the tap tables in
//!   `lighting.wgsl`, a prefix of the row-major nine sits off-centre by up to
//!   two thirds of the light's radius.
//! * The soft band must widen in proportion to (receiver − caster), which is
//!   similar-triangles arithmetic. Both distances come from one render, so
//!   nothing else is allowed to differ between them.
//!
//! Two things are measured rather than assumed, and each was a bug in this
//! file's first draft. The **axis** comes from the same scene lit by a
//! point-sized light, whose shadow is symmetric about the true axis by
//! construction, so the readback's half-pixel convention lands there instead of
//! being mistaken for a tap bias. The **falloff** is cancelled by dividing the
//! shadowed column by the unshadowed one, row for row: normalising by a
//! column's brightest byte instead lets the light's own falloff cross the
//! silhouette level several pixels before any shadow does, which inflated the
//! measured shadow by a quarter at every rung above two while leaving the point
//! light — whose level sits far below the falloff — looking flawless.
//!
//! The per-pixel rotation of the tap set is doing its job, so a profile is not
//! smooth: neighbours differ by one tap, a ninth at nine rays. Every tolerance
//! here comes from what `the_penumbra_profiles_print_themselves` reports — read
//! that before changing a number in this file.

use super::super::*;
use super::shadows::SPAN;
use super::shadows::paint;
use super::*;
use crate::Canvas;
use crate::canvas::Occluder;
use crate::collision::{Collider, Convex, OrientedBox};
use crate::objects;
use crate::objects::*;

/// The scene's one caster: 2 px half-extent along the light's axis, 6 px across
/// it, centred on the axis at x = 24. Six against a 3 px disk, so an umbra
/// exists — with a disk wider than the caster no point behind it is dark, and
/// the first draft of this scene had that shape: the darkest pixel read 0.22
/// and no assertion would have made it zero.
const CASTER: [f32; 2] = [2.0, 6.0];

/// What stands between the light and the surface.
///
/// Two casters because the engine has two roads into the occluder field. A box
/// is *painted* here — an invisible draw whose `occludes` flag is the only thing
/// the field is built from — and a polygon is *declared* through the canvas,
/// because a polygon's edges are its shadow and only the declare channel carries
/// them. [`super::shadows`] proves the two roads reach the field as the same
/// record for a circle; [`super::polygon`] points the meter below at the second
/// one, which is where the plane walk of §22 and §23 lives and where an
/// optimisation that drops a ray has somewhere to be caught.
// Boxed? No: this enum is built a handful of times per test run and only ever
// lives inside one function, and a pointer would sit between the reader and the
// shape being read about.
#[allow(clippy::large_enum_variant)]
pub(super) enum Caster {
    /// A box of these half-extents, centred on the axis at x = 24.
    Box([f32; 2]),
    /// A convex polygon, centred on the axis at x = 24.
    Polygon(Convex),
}
/// The light's disk radius, and the axis its centre sits on.
const DISK: f32 = 3.0;
const AXIS: f32 = 32.0;

/// One column of the scene, shadowed or lit alone. `shadowed` false drops the
/// caster from the draw list and leaves the light, the surface and their
/// falloff exactly as they were, so the ratio of the two is the blocked
/// fraction with the falloff cancelled — not by assumption, but by dividing by
/// the frame that measured it.
pub(super) fn plane(caster: &Caster, penumbra: f32, taps: u32, shadowed: bool) -> Vec<u8> {
    let surface = Draw::Shape {
        world: Transform::identity(),
        center: [32.0, AXIS],
        params: [80.0, 80.0],
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        emission: black(),
        glow: None,
        lit: 1.0,
        occludes: 0.0,
        z: 0.0,
        diagnostic: false,
        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    // Alpha 0: the caster composites nothing, so the byte read back is light
    // and nothing else, while the occluder field still carries it.
    let box_caster = Draw::Shape {
        world: Transform::identity(),
        center: [24.0, AXIS],
        params: CASTER,
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.0,
        },
        emission: black(),
        glow: None,
        lit: 0.0,
        occludes: 1.0,
        z: 1.0,
        diagnostic: false,
        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let light = Draw::Light {
        pos: [8.0, AXIS],
        radius: 1000.0,
        intensity: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        penumbra,
        dir: [1.0, 0.0],
        cos_half: -1.0,
        // An omni light's feather slot carries the tap rung: that is how the
        // shader learns how many rays sweep the disk.
        feather: taps as f32,
        z: 0.0,
        layers: LightLayers::default(),
    };
    let behind: Vec<Draw> = match caster {
        Caster::Box(_) => vec![box_caster],
        Caster::Polygon(_) => Vec::new(),
    };
    let declared: Vec<Occluder> = match (caster, shadowed) {
        (Caster::Box(_), _) | (Caster::Polygon(_), false) => Vec::new(),
        (Caster::Polygon(polygon), true) => {
            let mut canvas = Canvas::new((SPAN, SPAN), 1.0);
            // The declare channel takes user space and folds the canvas's own
            // y-flip into it, while the light and surface above are named in
            // pixel space. The map at scale 1 is pixel = (32 + x, 32 − y), so
            // the caster's post on the axis at pixel x = 24 is user (−8, 0).
            canvas.occluder_polygon(Transform::translate([-8.0, 0.0]), polygon);
            canvas.occluders
        }
    };
    let mut draws = vec![light, surface];
    if shadowed {
        draws.extend(behind);
    }
    paint(&draws, &declared).expect("this test needs an adapter")
}

/// One column of [`plane`]'s target, as bytes per row: what every measurement in
/// this file walks, and what [`super::polygon`] fingerprints a step before this.
pub(super) fn column(
    caster: &Caster,
    penumbra: f32,
    taps: u32,
    probe_x: u32,
    shadowed: bool,
) -> Vec<f32> {
    let red = plane(caster, penumbra, taps, shadowed);
    (0..SPAN)
        .map(|y| f32::from(red[(y as usize * SPAN as usize) + probe_x as usize]))
        .collect()
}

/// The caster as the CPU can answer for it, in the space each kind is held in.
///
/// This is the oracle that needs no model of the tap set at all. The engine's
/// rays start somewhere inside a region of radius `reach` about the lamp — the
/// same number the whole-disk reject inflates occluders by — so:
///
/// * if **no** origin in that region is blocked, the row must read fully lit,
///   whatever points the ladder happened to sample and however it was spun; and
/// * if **every** origin in it is blocked, the row must read dark.
///
/// Both directions are sound for a *subset* of the region, which is what nine
/// taps are, so the test holds for any tap count, any order and any rotation —
/// the property a per-pixel CPU twin could not offer (§20). Over-blocking from a
/// reject or an angular wedge that discards a caster the rays do touch shows up
/// on the first; under-blocking on the second.
pub(super) struct CpuCaster {
    box_: Option<OrientedBox>,
    polygon: Option<Convex>,
    to_local: Transform,
}

impl CpuCaster {
    pub(super) fn new(caster: &Caster) -> Self {
        match caster {
            Caster::Box(half) => Self {
                box_: Some(OrientedBox::rotated([24.0, AXIS], *half, 0.0)),
                polygon: None,
                to_local: Transform::identity(),
            },
            Caster::Polygon(polygon) => {
                let mut canvas = Canvas::new((SPAN, SPAN), 1.0);
                canvas.occluder_polygon(Transform::translate([24.0 - AXIS, 0.0]), polygon);
                Self {
                    box_: None,
                    polygon: Some(*polygon),
                    to_local: canvas.occluders[0]
                        .world
                        .invert()
                        .expect("a non-degenerate caster"),
                }
            }
        }
    }

    /// Whether the segment from a lamp-side origin to a pixel meets the caster.
    pub(super) fn hits(&self, from: [f32; 2], to: [f32; 2]) -> bool {
        match (&self.box_, &self.polygon) {
            (Some(b), _) => Collider::Box(*b).occludes(from, to),
            (None, Some(p)) => p.occludes(self.to_local.apply(from), self.to_local.apply(to)),
            (None, None) => false,
        }
    }
}

/// The half-width a run of CPU-shaded rows describes, in the same continuous
/// units the interpolated crossings are: the run covers rows `first` through
/// `last`, which spans `first` to `last + 1` in pixel space.
fn cpu_half(first: Option<u32>, last: Option<u32>) -> f32 {
    match (first, last) {
        (Some(f), Some(l)) => (l + 1 - f) as f32 / 2.0,
        _ => 0.0,
    }
}

/// The origins the taps can be drawn from: the centre plus four rings of
/// twenty-four out to `reach`. A grid, not the nine, because the ladder is
/// rotated per pixel and an assertion may not depend on where it landed.
fn origins(reach: f32) -> Vec<[f32; 2]> {
    let lamp = [8.0, AXIS];
    if reach <= 0.0 {
        return vec![lamp];
    }
    let mut out = vec![lamp];
    for ring in 1..=4 {
        let r = reach * ring as f32 / 4.0;
        for k in 0..24 {
            let a = std::f32::consts::TAU * k as f32 / 24.0;
            out.push([lamp[0] + r * a.cos(), lamp[1] + r * a.sin()]);
        }
    }
    out
}

/// The rows a column must read lit, and the rows it must read dark, from the
/// CPU alone. `reach` is the radius the taps are drawn from at this rung:
/// nothing at one ray, one penumbra through five, a corner's √2 above that.
pub(super) fn verdict(caster: &Caster, probe_x: u32, reach: f32) -> (Vec<u32>, Vec<u32>) {
    let cpu = CpuCaster::new(caster);
    let rays = origins(reach);
    let mut never = vec![];
    let mut always = vec![];
    for y in 0..SPAN {
        let to = [probe_x as f32 + 0.5, y as f32 + 0.5];
        let blocked = rays.iter().filter(|o| cpu.hits(**o, to)).count();
        if blocked == 0 {
            never.push(y);
        } else if blocked == rays.len() {
            always.push(y);
        }
    }
    (never, always)
}

/// The reach this rung draws its taps from, in the same units as the penumbra.
pub(super) fn reach_of(taps: u32, penumbra: f32) -> f32 {
    match taps {
        1 => 0.0,
        2..=5 => penumbra,
        _ => penumbra * 2.0_f32.sqrt(),
    }
}

/// The lit fraction down one column: shadowed over unshadowed, row for row.
/// 1.0 is every ray arriving, 0.0 is none of them.
pub(super) fn profile(penumbra: f32, taps: u32, probe_x: u32) -> Vec<f32> {
    profile_with(&Caster::Box(CASTER), penumbra, taps, probe_x)
}

/// The same meter on any caster: this is the whole reason the box scene's
/// profile is a function rather than a fixture.
pub(super) fn profile_with(caster: &Caster, penumbra: f32, taps: u32, probe_x: u32) -> Vec<f32> {
    let lit = column(caster, penumbra, taps, probe_x, false);
    let shaded = column(caster, penumbra, taps, probe_x, true);
    lit.iter()
        .zip(&shaded)
        .map(|(l, s)| if *l > 1.0 { s / l } else { 1.0 })
        .collect()
}

/// Walk in from `from` towards `toward` and report where the profile first
/// falls below `level`, interpolated between whole pixel rows.
fn first_drop(p: &[f32], level: f32, from: usize, toward: isize) -> Option<f32> {
    let mut prev = p[from];
    let mut i = from as isize;
    loop {
        i += toward;
        if i < 0 || i as usize >= p.len() {
            return None;
        }
        let now = p[i as usize];
        if now < level {
            // The two samples bracketing the crossing are `now` at row i and
            // `prev` at row i − toward. Walking down, that interval is (i−1, i)
            // and the crossing is i − 1 + frac; walking *up*, it is (i, i+1) and
            // the crossing is i + 1 − frac. The line used to be the downhill
            // case whatever the direction, so every lower flank this meter
            // measured came out one row high on the shadow — found by pointing
            // it at a polygon, whose hard edge the collision code puts on known
            // rows, and watching the two disagree by exactly a pixel.
            let frac = (prev - level) / (prev - now);
            return Some(i as f32 - toward as f32 * (1.0 - frac));
        }
        prev = now;
    }
}

/// Half a tap below full light: the level that catches *one ray lost*, which is
/// the outline of the shadow rather than the edge of its dark core, and means
/// the same thing on a rung of two as on a rung of nine.
fn outline_level(taps: u32) -> f32 {
    1.0 - 0.5 / taps as f32
}

/// Where the shadow begins on each flank, walking in from the full light at the
/// top and at the bottom of the column. Both flanks must use the same criterion:
/// the first draft looked for a fall through half light on one side and a rise
/// through it on the other, so on a coarse rung the two flanks answered
/// different questions and a symmetric shadow reported itself lopsided.
pub(super) fn silhouette(p: &[f32], taps: u32) -> (f32, f32) {
    let level = outline_level(taps);
    // `first_drop` running off the end means the column never darkened, not that
    // it is dark: the message said "all shadow" here and sent one debugging run
    // the wrong way through a scene whose reference frame was the one at fault.
    let top = first_drop(p, level, 0, 1).expect("no upper silhouette: the column is all light");
    let bottom = first_drop(p, level, (SPAN - 1) as usize, -1)
        .expect("no lower silhouette: the column is all light");
    (top, bottom)
}

/// The axis as the machine sees it: the silhouette centre of the same scene lit
/// by a point-sized light, whose shadow is symmetric about the axis by
/// construction. It lands away from 32, and measuring it is what keeps the
/// harness's pixel convention out of the tap-bias numbers.
fn measured_axis(probe_x: u32) -> f32 {
    measured_axis_with(&Caster::Box(CASTER), probe_x)
}

/// The axis for another caster, measured the same way — from that caster's own
/// point-sized render, so the harness's half-pixel convention lands in the
/// number instead of being mistaken for a bias in the tap set.
pub(super) fn measured_axis_with(caster: &Caster, probe_x: u32) -> f32 {
    let (t, b) = silhouette(&profile_with(caster, 0.0, 1, probe_x), 1);
    (t + b) / 2.0
}

/// The soft band on the lower flank: from the silhouette in, to where the umbra
/// begins. The umbra does not scale with distance the way the band does, so the
/// two edges are measured separately — the distance between the two flanks'
/// silhouettes is mostly umbra, and a ratio taken of that is wrong.
pub(super) fn band(p: &[f32], taps: u32) -> f32 {
    let start = (SPAN - 1) as usize;
    let out = first_drop(p, outline_level(taps), start, -1).expect("no silhouette on this flank");
    let dark = first_drop(p, 0.02, start, -1)
        .expect("no umbra on this flank: the disk has to be narrower than the caster");
    out - dark
}

/// Where the umbra begins on each flank: the last ray to be blocked, walking in
/// from full light. The silhouette is set by the outermost tap and so stays
/// centred even when the sample set is off-centre; the dark core is where every
/// tap has to be blocked, and that is what an off-centre set moves.
pub(super) fn core(p: &[f32]) -> (f32, f32) {
    let top = first_drop(p, 0.02, 0, 1).expect("no umbra above the axis");
    let bottom = first_drop(p, 0.02, (SPAN - 1) as usize, -1).expect("no umbra below the axis");
    (top, bottom)
}

/// The dark half of a column: how many pixel-rows' worth of light the caster
/// takes away, counting a row that loses a third of its light as a third.
///
/// Written here first as an invariant — each ray sweeps a hard shadow as wide as
/// the point light's, only displaced, so the total cannot depend on how the disk
/// is sampled. *Only displaced* is the part that is only true in parallel
/// projection. Under this one, a tap nearer the caster casts a wider shadow, and
/// the ladder's five-point-in-three-radii weighting is not a quadrature of the
/// disk; [`super::polygon`] measures the result on a sixteen-sided caster, where
/// the same total moves 16.00 → 17.50 rows between one ray and six. On this box
/// the whole spread is a quarter of a row, so the bound below still holds and
/// still catches a ray going missing outright — but it is a bound on jitter now,
/// not the law it was described as, and the row-for-row oracle two screens up is
/// the instrument that has no premise of that kind to fail.
pub(super) fn darkness(p: &[f32]) -> f32 {
    p.iter().map(|v| 1.0 - v).sum()
}

/// Half the distance between the umbra's two flanks.
pub(super) fn core_half(p: &[f32]) -> f32 {
    let (t, b) = core(p);
    (b - t) / 2.0
}

/// Every row of the two probe columns, for reading the shape of a profile
/// instead of inferring it from crossings.
#[test]
#[ignore]
fn the_penumbra_dumps_its_columns() {
    for taps in [2u32, 9] {
        for probe_x in [40u32, 56] {
            let p = profile(DISK, taps, probe_x);
            let flat: Vec<String> = p
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{i}:{v:.2}"))
                .collect();
            println!("probe {probe_x} taps {taps}: {}", flat.join(" "));
        }
    }
}

/// The printout every tolerance in this file was set from. Run it when anything
/// touches the tap tables or the disk, and read it before believing a number.
#[test]
#[ignore]
fn the_penumbra_profiles_print_themselves() {
    for probe_x in [40u32, 56] {
        let axis = measured_axis(probe_x);
        let hard = profile(0.0, 1, probe_x);
        let (ht, hb) = silhouette(&hard, 1);
        let (_, always) = verdict(&Caster::Box(CASTER), probe_x, 0.0);
        println!(
            "probe {probe_x}: axis {axis:.2}, point-light half-width {:.2} (the CPU walk puts it at \
             {:.2})",
            (hb - ht) / 2.0,
            cpu_half(always.first().copied(), always.last().copied()),
        );
        for taps in 1..=9u32 {
            let p = profile(DISK, taps, probe_x);
            let (t, b) = silhouette(&p, taps);
            let (ct, cb) = core(&p);
            let (never, _) = verdict(&Caster::Box(CASTER), probe_x, reach_of(taps, DISK));
            let shadable: Vec<u32> = (0..SPAN).filter(|y| !never.contains(y)).collect();
            println!(
                "  taps {taps}: silhouette {t:.2}..{b:.2} half {:.2} (cpu union \
                 {}..{}) | core half {:.2} mid {:+.2} off axis | darkness {:.2} rows",
                (b - t) / 2.0,
                shadable.first().copied().unwrap_or(0),
                shadable.last().copied().unwrap_or(SPAN - 1),
                core_half(&p),
                (ct + cb) / 2.0 - axis,
                darkness(&p),
            );
        }
    }
}

/// Deep umbra, open light: what the geometry guarantees however the disk is
/// sampled, and the check that the scene means what it claims. The caster is
/// 6 px across the axis against a 3 px disk, so a pixel on the axis behind it
/// sees no part of the light, and a pixel clear of the silhouette sees all of it.
#[test]
fn a_penumbra_is_dark_behind_the_caster_and_full_outside_it() {
    // Probe 40 rather than 56. A 3 px disk throwing a 6 px caster's shadow 32 px
    // past the caster covers 60 of the 64 rows at the far probe, so hardly any
    // unshadowed column is left out there to compare against; here the
    // silhouette reaches 19.5 px each side and leaves rows 4 and 60 in open light.
    let p = profile(DISK, 9, 40);
    assert!(p[28] < 0.02 && p[34] < 0.02, "the umbra is not dark: {p:?}");
    assert!(
        p[4] > 0.98 && p[60] > 0.98,
        "open light is not fully lit: {p:?}"
    );
}

/// The hard shadow, measured against nothing but similar triangles: half-width
/// = caster half-height ÷ (near edge of the caster − light) × (probe − light).
/// Both probes land inside a quarter pixel of it — 13.50 against 13.71 and
/// 20.50 against 20.57 — which is the coordinate contract of this harness
/// (readback order, light record layout, occluder-space `m`) confirmed by
/// arithmetic rather than by a previous build.
#[test]
fn a_point_lights_shadow_is_where_triangles_put_it() {
    for probe_x in [40u32, 56] {
        let (t, b) = silhouette(&profile(0.0, 1, probe_x), 1);
        let half = (b - t) / 2.0;
        let (_, always) = verdict(&Caster::Box(CASTER), probe_x, 0.0);
        let want = cpu_half(always.first().copied(), always.last().copied());
        assert!(
            (half - want).abs() <= 0.5,
            "probe {probe_x}: the point light's shadow is {half:.2} px wide either side, where the \
             triangles say {want:.2}"
        );
    }
}

/// The outer edge of a soft shadow is the shadow of the disk's rim: a tap at
/// lateral distance `reach` whose ray just clips the caster's near edge. The
/// cross reaches one radius, the grid √2 radii, because its corners are there.
/// Measured 27.50 against 27.86 and 29.99 against 30.87 — and the residual is
/// one-sided by construction: along a column the rotation rarely puts a tap
/// exactly on the rim, so a measured silhouette can fall short of the rim's
/// **The outline sits inside what the disk can actually shade, and outside the
/// point light's own shadow.** The CPU oracle answers row by row which rows some
/// origin within the tap region can reach; no measured edge may go past that. And
/// the centre ray is in every rung of the ladder, so the outline can never be
/// narrower than the hard shadow either. Both bounds come from the engine's own
/// collision code rather than from a model of the disk.
///
/// This replaces `rim()`, an analytic formula for the outermost ray, and the
/// reason is worth keeping. The formula treated a tap's reach as a purely
/// lateral offset — (half + reach·(1−u))/u — but a tap displaced *along* the axis
/// widens the shadow too, since an origin nearer the caster subtends it at a
/// steeper angle, and at nine rays, where the corners sit √2 out, that omission
/// came to about a pixel. The formula and this file's off-by-one on the lower
/// flank were each about half a pixel low, which is the whole reason they agreed
/// so neatly for as long as they did: fixing the meter broke the agreement, and
/// the oracle — which models nothing, it just asks — showed the shadow sitting
/// exactly where the geometry puts it. So the formula is deleted rather than
/// widened. A bound the collision code can answer directly should not be a
/// formula.
#[test]
fn a_soft_shadows_outer_edge_stays_inside_the_disks_union() {
    for probe_x in [40u32, 56] {
        let (hard_top, hard_bottom) = silhouette(&profile(0.0, 1, probe_x), 1);
        for taps in 1..=9u32 {
            let p = profile(DISK, taps, probe_x);
            let (t, b) = silhouette(&p, taps);
            let (never, _) = verdict(&Caster::Box(CASTER), probe_x, reach_of(taps, DISK));
            let shadable: Vec<u32> = (0..SPAN).filter(|y| !never.contains(y)).collect();
            let (first, last) = (
                shadable.first().copied().unwrap_or(0) as f32,
                shadable.last().copied().unwrap_or(SPAN - 1) as f32,
            );
            assert!(
                t >= first - 1.0 && b <= last + 1.0,
                "taps {taps} at probe {probe_x}: the outline spans {t:.2}..{b:.2} but the CPU says \
                 nothing inside the tap region can shade below {first:.0} or above {last:.0} — \
                 rays are being blocked that never touch the caster",
            );
            assert!(
                t <= hard_top + 1.0 && b >= hard_bottom - 1.0,
                "taps {taps} at probe {probe_x}: the outline spans {t:.2}..{b:.2}, narrower than the \
                 point light's {hard_top:.2}..{hard_bottom:.2}, although the centre ray is in every \
                 rung and casts that shadow wherever it casts one",
            );
        }
    }
}

/// One ray is a point light, and a point light has no penumbra: the band
/// between the silhouette and the umbra must be nothing, and the dark core must
/// be the whole shadow. Measured exactly: 54.00 px of silhouette, 54.00 px of
/// core, a band of −0.48 px of interpolation slop.
///
/// And since the ladder starts at the disk's centre, one ray is not merely
/// penumbra-less, it is *the point light*: the same silhouette to a quarter
/// pixel, 13.50 against 13.50 and 20.50 against 20.50. Before the ladder was
/// rebalanced the first tap was a rim point spun to a random angle per pixel, so
/// `taps: 1` meant a hard shadow that wandered by up to a penumbra radius —
/// which is what this test now forbids.
#[test]
fn one_ray_therefore_no_penumbra_at_all() {
    for probe_x in [40u32, 56] {
        let p = profile(DISK, 1, probe_x);
        let (t, b) = silhouette(&p, 1);
        let (ht, hb) = silhouette(&profile(0.0, 1, probe_x), 1);
        assert!(
            (b - t - (hb - ht)).abs() <= 0.25,
            "one ray casts a shadow {t:.2}..{b:.2} where the point light casts {ht:.2}..{hb:.2}: one \
             ray from the disk's centre should be that ray and no more",
        );
        let soft = band(&p, 1);
        assert!(
            soft.abs() <= 1.0,
            "one ray left a {soft:.2} px soft band between silhouette {t:.2} and the umbra: a single \
             ray cannot average anything"
        );
        let (ct, cb) = core(&p);
        assert!(
            (cb - ct - (b - t)).abs() <= 1.0,
            "one ray: core {:.2} px against silhouette {:.2} px — the core of a single ray's shadow \
             is that shadow",
            cb - ct,
            b - t
        );
    }
}

/// How much light the caster eats must not depend on how many rays are asked
/// for. Each ray carries a hard shadow of its own, so summing the fraction lost
/// over the column counts the same shadow whichever rays drawn it — the only
/// thing the rung changes is where the estimate wobbles, not how much it totals.
/// Measured spread: 27.34 to 28.59 rows at probe 40, 42.00 to 43.50 at probe 56,
/// over rungs 2 through 9.
///
/// The point light is *not* in this comparison, and that is geometry rather
/// than error: a disk extends along the axis toward the caster, and a ray nearer
/// the caster magnifies more, so a wide disk eats slightly more light than a
/// point does — 28.0 rows against 27.0 at probe 40, three and a half percent.
#[test]
fn the_darkness_a_casters_casts_does_not_depend_on_the_tap_count() {
    for probe_x in [40u32, 56] {
        let rows: Vec<f32> = (2..=9u32)
            .map(|taps| darkness(&profile(DISK, taps, probe_x)))
            .collect();
        let least = rows.iter().copied().fold(f32::INFINITY, f32::min);
        let most = rows.iter().copied().fold(0.0f32, f32::max);
        // Probe 40 holds the whole shadow inside the target and can be strict.
        // Probe 56 cannot: the widest rungs reach row 1.5 and row 61.5 of 64, so
        // their tails fall off the edge of the readback and the sum loses them.
        let slack = if probe_x == 40 { 1.05 } else { 1.10 };
        assert!(
            most / least <= slack,
            "probe {probe_x}: rungs 2..9 ate {rows:?} rows of light; the same scene must not care \
             how many rays draw its shadow, and these differ by {:.0}%",
            (most / least - 1.0) * 100.0
        );
    }
}

/// Whatever the rung, the shadow's outline must be centred on the light's own
/// axis, which is measured from a point light rather than assumed: the readback
/// puts it at 31.0 in this harness, and every number here is relative to that.
/// The worst measured rung is 1.38 px out, which is the dither a single column carries. The fine rungs land within 0.90 px,
/// because a ninth of the band moves the level crossing a little; `taps: 2` was
/// measured 1.87 px out at probe 40 and cannot do better, since a two-ray column
/// has exactly three levels — lit, half lit, dark — so its silhouette is one coin
/// flip per row, thrown by a rotation that advances
/// 111° per row down a column and 200° per pixel across one. The claim that
/// survives the coin flip is the umbra's, below, and the ladder's monotonicity.
#[test]
fn every_rung_keeps_the_shadow_centred_on_the_light() {
    for probe_x in [40u32, 56] {
        let axis = measured_axis(probe_x);
        for taps in 1..=9u32 {
            let p = profile(DISK, taps, probe_x);
            let (t, b) = silhouette(&p, taps);
            let off = (t + b) / 2.0 - axis;
            assert!(
                off.abs() <= 2.0,
                "taps {taps} at probe {probe_x}: silhouette {t:.2}..{b:.2} sits {off:+.2} px off the \
                 axis a point light put at {axis:.2}"
            );
        }
    }
}

/// The same for the dark core, with room for what a coarse rung genuinely
/// cannot resolve. The bound is the measured worst (2.48 px, at `taps: 1` on
/// probe 40) plus slack, and it earns its place by what it *refutes*: the tap
/// tables' off-centre prefixes were expected to drag the edge of the penumbra
/// by up to two thirds of a radius, several pixels here. They do not. The
/// rotation is random per pixel, so a set whose mean sits off the light's centre
/// is spun in a different direction in every pixel, and the displacement does
/// not add up — it lands as jitter inside the ±2 px above, with its sign
/// different between the two probe columns. What the prefix defect does leave
/// behind is the next test, not this one.
#[test]
fn every_rung_keeps_the_umbra_near_the_axis() {
    for probe_x in [40u32, 56] {
        let axis = measured_axis(probe_x);
        for taps in 1..=9u32 {
            let p = profile(DISK, taps, probe_x);
            let (ct, cb) = core(&p);
            let off = (ct + cb) / 2.0 - axis;
            assert!(
                off.abs() <= 2.5,
                "taps {taps} at probe {probe_x}: umbra {ct:.2}..{cb:.2} sits {off:+.2} px off the axis \
                 at {axis:.2}"
            );
        }
    }
}

/// More rays must not make the core shadow grow. For a given pixel the disk
/// points whose ray is blocked form a convex set — the slab test narrows an
/// interval, so an origin blocked on two counts blocks every origin between them
/// — and the tap ladder is nested, so rung N+1 surveys a superset of the points
/// rung N surveyed. A pixel dark at N+1 rays was therefore dark at N, and the
/// umbra can only shrink as rays are added. Measured half-widths, rungs 1
/// through 9: 13.50, 13.49, 11.50, 10.98, 10.00, 9.47, 9.00, 9.00, 8.50 at probe
/// 40, and 20.50, 20.49, 18.00, 16.98, 13.50, 13.53, 12.97, 12.46, 11.50 at probe
/// 56. It never grows; the +0.03 px between five and six rays is the interpolation
/// walking across a step, which is why the bound is a third of a pixel, not zero.
///
/// This is the property the ladder was built for. The row-major order it replaced
/// asked for five rays and received a prefix holding no point on one side of the
/// disk at all, so the fifth rung's core shadow measured 4.96 px *wider* than the
/// fourth's and a user raising the tap count watched the umbra grow (§28). If this
/// test fails, the tap table has stopped being a nested ladder — and it should
/// fail, because that growth is the one artefact a tap count is not allowed to
/// have.
#[test]
fn more_rays_never_widen_the_umbra() {
    for probe_x in [40u32, 56] {
        let widths: Vec<f32> = (1..=9u32)
            .map(|taps| core_half(&profile(DISK, taps, probe_x)))
            .collect();
        for (i, pair) in widths.windows(2).enumerate() {
            assert!(
                pair[1] <= pair[0] + 0.34,
                "probe {probe_x}: the umbra is {:.2} px wide at {} rays and {:.2} px at {} — a finer \
                 sample of one disk can only find rays that escape, never lose one: {widths:?}",
                pair[0],
                i + 1,
                pair[1],
                i + 2
            );
        }
        assert!(
            widths[8] < widths[0],
            "probe {probe_x}: nine rays leave a {pop} px umbra where one ray left {first} px. The \
             ladder is nested, so the nine-ray core shadow must sit inside the single ray's",
            pop = widths[8],
            first = widths[0]
        );
    }
}
