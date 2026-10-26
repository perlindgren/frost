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
use crate::canvas::Occluder;
use crate::objects;
use crate::objects::*;

/// The scene's one caster: 2 px half-extent along the light's axis, 6 px across
/// it, centred on the axis at x = 24. Six against a 3 px disk, so an umbra
/// exists — with a disk wider than the caster no point behind it is dark, and
/// the first draft of this scene had that shape: the darkest pixel read 0.22
/// and no assertion would have made it zero.
const CASTER: [f32; 2] = [2.0, 6.0];
/// The light's disk radius, and the axis its centre sits on.
const DISK: f32 = 3.0;
const AXIS: f32 = 32.0;

/// One column of the scene, shadowed or lit alone. `shadowed` false drops the
/// caster from the draw list and leaves the light, the surface and their
/// falloff exactly as they were, so the ratio of the two is the blocked
/// fraction with the falloff cancelled — not by assumption, but by dividing by
/// the frame that measured it.
fn column(penumbra: f32, taps: u32, probe_x: u32, shadowed: bool) -> Vec<f32> {
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
    let caster = Draw::Shape {
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
    let none: Vec<Occluder> = Vec::new();
    let draws: Vec<Draw> = if shadowed {
        vec![light, surface, caster]
    } else {
        vec![light, surface]
    };
    let red = paint(&draws, &none).expect("this test needs an adapter");
    (0..SPAN)
        .map(|y| f32::from(red[(y as usize * SPAN as usize) + probe_x as usize]))
        .collect()
}

/// The lit fraction down one column: shadowed over unshadowed, row for row.
/// 1.0 is every ray arriving, 0.0 is none of them.
fn profile(penumbra: f32, taps: u32, probe_x: u32) -> Vec<f32> {
    let lit = column(penumbra, taps, probe_x, false);
    let shaded = column(penumbra, taps, probe_x, true);
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
            return Some(i as f32 - 1.0 + (prev - level) / (prev - now));
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
fn silhouette(p: &[f32], taps: u32) -> (f32, f32) {
    let level = outline_level(taps);
    let top = first_drop(p, level, 0, 1).expect("no upper silhouette: the column is all shadow");
    let bottom = first_drop(p, level, (SPAN - 1) as usize, -1)
        .expect("no lower silhouette: the column is all shadow");
    (top, bottom)
}

/// The axis as the machine sees it: the silhouette centre of the same scene lit
/// by a point-sized light, whose shadow is symmetric about the axis by
/// construction. It lands away from 32, and measuring it is what keeps the
/// harness's pixel convention out of the tap-bias numbers.
fn measured_axis(probe_x: u32) -> f32 {
    let (t, b) = silhouette(&profile(0.0, 1, probe_x), 1);
    (t + b) / 2.0
}

/// The soft band on the lower flank: from the silhouette in, to where the umbra
/// begins. The umbra does not scale with distance the way the band does, so the
/// two edges are measured separately — the distance between the two flanks'
/// silhouettes is mostly umbra, and a ratio taken of that is wrong.
fn band(p: &[f32], taps: u32) -> f32 {
    let start = (SPAN - 1) as usize;
    let out = first_drop(p, outline_level(taps), start, -1).expect("no silhouette on this flank");
    let dark = first_drop(p, 0.02, start, -1)
        .expect("no umbra on this flank: the disk has to be narrower than the caster");
    out - dark
}

/// The half-width the geometry promises for the outermost ray of the disk:
/// the tap at lateral distance `reach` whose segment just clips the caster's
/// near edge. The near edge, not the far one — the first draft grazed the far
/// edge and predicted a shadow a quarter narrower than the one measured, which
/// is how this file found that it had the wrong model rather than that the
/// engine had the wrong shadow.
fn rim(probe_x: u32, reach: f32) -> f32 {
    let u = (22.0 - 8.0) / (probe_x as f32 - 8.0);
    (6.0 + reach * (1.0 - u)) / u
}

/// Where the umbra begins on each flank: the last ray to be blocked, walking in
/// from full light. The silhouette is set by the outermost tap and so stays
/// centred even when the sample set is off-centre; the dark core is where every
/// tap has to be blocked, and that is what an off-centre set moves.
fn core(p: &[f32]) -> (f32, f32) {
    let top = first_drop(p, 0.02, 0, 1).expect("no umbra above the axis");
    let bottom = first_drop(p, 0.02, (SPAN - 1) as usize, -1).expect("no umbra below the axis");
    (top, bottom)
}

/// The dark half of a column: how many pixel-rows' worth of light the caster
/// takes away, counting a row that loses a third of its light as a third. By
/// Fubini this is the same however the disk is sampled — each ray sweeps a
/// hard shadow of its own, and a ray's shadow is as wide as the point light's
/// whatever the ray's offset, only moved — so this number must not depend on
/// the penumbra or the tap count. It is the strongest check here and it needs
/// no model of the disk at all, only the point light's own width.
fn darkness(p: &[f32]) -> f32 {
    p.iter().map(|v| 1.0 - v).sum()
}

/// Half the distance between the umbra's two flanks.
fn core_half(p: &[f32]) -> f32 {
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
        println!(
            "probe {probe_x}: axis {axis:.2}, point-light half-width {:.2} (geometry says {:.2})",
            (hb - ht) / 2.0,
            rim(probe_x, 0.0)
        );
        for taps in 1..=9u32 {
            let p = profile(DISK, taps, probe_x);
            let (t, b) = silhouette(&p, taps);
            let (ct, cb) = core(&p);
            // The ladder's own reach: nothing at one tap, one radius through
            // five, a corner's sqrt(2) above that.
            let reach = match taps {
                1 => 0.0,
                2..=5 => DISK,
                _ => DISK * 2.0_f32.sqrt(),
            };
            println!(
                "  taps {taps}: silhouette {t:.2}..{b:.2} half {:.2} vs rim {:.2} | core half {:.2} \
                 mid {:+.2} off axis | darkness {:.2} rows",
                (b - t) / 2.0,
                rim(probe_x, reach),
                core_half(&p),
                (ct + cb) / 2.0 - axis,
                darkness(&p)
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
        let want = rim(probe_x, 0.0);
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
/// shadow, never exceed it.
#[test]
fn a_soft_shadows_outer_edge_is_the_shadow_of_the_disks_rim() {
    for probe_x in [40u32, 56] {
        for (taps, reach) in [(4u32, DISK), (9u32, DISK * 2.0_f32.sqrt())] {
            let p = profile(DISK, taps, probe_x);
            let (t, b) = silhouette(&p, taps);
            let half = (b - t) / 2.0;
            let want = rim(probe_x, reach);
            assert!(
                (half - want).abs() <= 1.5,
                "taps {taps} at probe {probe_x}: silhouette half-width {half:.2} px against the rim's \
                 {want:.2} (reach {reach:.2} px)"
            );
            assert!(
                half <= want + 0.6,
                "taps {taps} at probe {probe_x}: the shadow is {half:.2} px wide, wider than its own \
                 rim permits at {want:.2}: rays are being blocked that never touch the caster"
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
