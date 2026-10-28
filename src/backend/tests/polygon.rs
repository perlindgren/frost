//! The polygon shadow, guarded for the optimisation that has not happened yet.
//!
//! §22 and §23 restructured the plane walk — the disk's rays against a polygon's
//! edges, which is the most expensive thing the engine does — and both sections
//! rest on the same assurance: *bit-for-bit the same picture*, three fingerprints
//! byte-identical to the committed build. Those sixteen-digit numbers are in the
//! prose of `docs/benchmarking.md` and nowhere else. `benchsuite` has no hashing,
//! no dump, no checksum; the tool that produced them is not in this repository.
//! So the claim that the two largest optimisations in the renderer preserved the
//! image cannot be re-checked by anyone, including whoever writes the next one —
//! and the next one is the angular wedge, which is aimed squarely at this path
//! because nine rays over sixty discs cost 23.67 ms against boxes' 4.86.
//!
//! This file puts a guard where the fingerprint was. Two layers, because they
//! answer different questions:
//!
//! * **Geometry**, in [`penumbra`]'s meter pointed at casters with edges. The
//!   CPU collision walk is the authority for the hard shadow — the same
//!   independent implementation [`shadows`] uses — and everything the disk adds
//!   is then bounded by properties the geometry forces: one ray is the point
//!   light, more rays never widen the umbra, the darkness a caster casts does not
//!   depend on how many rays swept it, the outline stays centred on the axis, and
//!   the soft edge stays inside what similar triangles allow. None of these needs
//!   a model of the tap set, which is the lesson §20 learned the hard way when a
//!   CPU twin disagreed with an unmodified shader on 692 of 4096 pixels.
//! * **Fingerprints**, pinned per scene. Exact to the byte, and honest about
//!   being a fact about this driver: they catch a change that leaves every
//!   property above intact, which is precisely what "bit-for-bit identical"
//!   promised. When the renderer changes *legitimately* the pinned numbers are
//!   wrong, and the remedy is deliberate — run the printer, read the geometry
//!   tests, re-pin.
//!
//! [`penumbra`]: super::penumbra
//! [`shadows`]: super::shadows

use super::penumbra::Caster;
use super::penumbra::{
    core_half, darkness, measured_axis_with, plane, profile_with, reach_of, silhouette, verdict,
};
use super::shadows::SPAN;
use crate::Canvas;
use crate::collision::{Circle, Collider, Convex};
use crate::objects::*;

/// The caster's radius, and the axis it is centred on. Four, not the box
/// scene's six across: the outer edge of a soft shadow reaches
/// `(R + disk·(1−u)) / u` from the axis, and at a probe column 48 px past a
/// 6 px caster that lands off the 64-px target, where the readback would simply
/// stop seeing the shadow and every number here would go quietly wrong.
const RADIUS: f32 = 4.0;
const AXIS: f32 = 32.0;
/// The light's disk radius, as in [`super::penumbra`]. Smaller than the caster
/// either way, which is what keeps an umbra behind it.
const DISK: f32 = 3.0;
/// Where the light sits and where the caster stands, both in pixel space, so
/// the geometry below is stated in the same units the readback is.
const LAMP: [f32; 2] = [8.0, AXIS];
const CASTER_X: f32 = 24.0;

/// The two probe columns: one where the shadow is comfortably inside the target
/// and one where the widest rungs come within eight pixels of its edge.
const NEAR_PROBE: u32 = 40;
const FAR_PROBE: u32 = 56;

/// A regular polygon of `sides` edges, centred on the origin — the hexagon §22
/// and §23 measured, built here rather than through the engine so that the
/// shape under test is the test's own claim.
pub(super) fn regular(sides: usize, radius: f32) -> Convex {
    let step = std::f32::consts::TAU / sides as f32;
    let points: Vec<[f32; 2]> = (0..sides)
        .map(|k| {
            let (sin, cos) = (k as f32 * step).sin_cos();
            [radius * cos, radius * sin]
        })
        .collect();
    Convex::new(&points).expect("a regular polygon is convex")
}

/// The scenes the walk is judged on: a hexagon, six edges, and a circle, which is
/// no polygon at all. They used to be a hexagon and a sixteen-sided disc — the
/// widest polygon the type allows, and what a drawn circle became on its way to the
/// field — and the pair was chosen to grade the walk against an approximation of
/// itself. The circle stays because the walk has to be judged against something it
/// is not: `Collider::Circle`, asked exactly, with nothing in between.
fn casters() -> Vec<(&'static str, Caster)> {
    vec![
        ("hexagon", Caster::Polygon(regular(6, RADIUS))),
        ("disc", Caster::Circle(RADIUS)),
    ]
}

/// The caster's own declared record, so the test asks the CPU in the frame the
/// GPU will use: the canvas builds the local-to-pixel map itself, including the
/// y-flip, so the test reads the map back off the record instead of recomputing
/// it and sharing the arithmetic it is meant to be checking.
fn declared(caster: &Caster) -> (Transform, Transform) {
    let mut canvas = Canvas::new((SPAN, SPAN), 1.0);
    match caster {
        Caster::Polygon(polygon) => {
            canvas.occluder_polygon(Transform::translate([CASTER_X - AXIS, 0.0]), polygon)
        }
        Caster::Circle(radius) => canvas.occluder_circle(
            Transform::translate([CASTER_X - AXIS, 0.0]),
            [0.0, 0.0],
            *radius,
        ),
        Caster::Box(_) => panic!("the box scene is penumbra.rs's, not this file's"),
    }
    let record = &canvas.occluders[0];
    (
        record.world,
        record.world.invert().expect("a non-degenerate caster"),
    )
}

/// The rows the caster blocks from a point light, walked by the collision code
/// rather than by the shader: `(top, bottom)` in pixels, from the first blocked
/// row to the last. `from_local` is the lamp in the caster's own space, where both
/// walks work — `Convex::occludes` for the edges, `Collider::Circle` for the
/// circle, which is collision's own answer and written without reference to the
/// shadow path. That is the point of the circle arm: the two engines are asked
/// about one shape and expected to agree exactly, with no tessellation between them
/// to grade the agreement on.
fn cpu_shadow(from_local: [f32; 2], to_local: &Transform, caster: &Caster) -> (f32, f32) {
    let circle = Collider::Circle(Circle {
        center: [0.0, 0.0],
        radius: match caster {
            Caster::Circle(radius) => *radius,
            _ => 0.0,
        },
    });
    let blocked = |y: u32| {
        // The probe column's centre in pixels; +0.5 is the sample convention
        // the shader uses and this file inherits from the meter.
        let p = to_local.apply([NEAR_PROBE as f32 + 0.5, y as f32 + 0.5]);
        match caster {
            Caster::Polygon(polygon) => polygon.occludes(from_local, p),
            _ => circle.occludes(from_local, p),
        }
    };
    let lit_rows: Vec<u32> = (0..SPAN).filter(|y| !blocked(*y)).collect();
    let dark: Vec<u32> = (0..SPAN).filter(|y| blocked(*y)).collect();
    assert!(
        !dark.is_empty() && !lit_rows.is_empty(),
        "the CPU walk says the caster covers {}/{} rows: nothing to compare",
        dark.len(),
        SPAN
    );
    (
        dark.first().copied().unwrap_or(0) as f32,
        dark.last().copied().unwrap_or(SPAN - 1) as f32 + 1.0,
    )
}

/// **The hard shadow is the collision code's shadow.** With the disk shut the
/// engine walks one ray and the CPU walks the same ray through the same edges,
/// so the two silhouettes must land on the same rows. This is the anchor: every
/// soft number below is measured relative to it, and if it drifts the polygon
/// path is wrong no matter what the fingerprints say.
#[test]
fn a_polygons_hard_shadow_is_the_one_the_collision_walk_finds() {
    for (name, caster) in casters() {
        let (to_pixel, to_local) = declared(&caster);
        let from_local = to_local.apply(LAMP);
        let (cpu_top, cpu_bottom) = cpu_shadow(from_local, &to_local, &caster);
        let (gpu_top, gpu_bottom) = silhouette(&profile_with(&caster, 0.0, 1, NEAR_PROBE), 1);
        assert!(
            (gpu_top - cpu_top).abs() <= 1.0 && (gpu_bottom - cpu_bottom).abs() <= 1.0,
            "{name}: the shader's hard shadow spans {gpu_top:.2}..{gpu_bottom:.2} where the \
             collision walk spans {cpu_top:.2}..{cpu_bottom:.2} — the plane list and the CPU's \
             half-planes have stopped agreeing, and no fingerprint can excuse that",
        );
        // The record has to land where this file says it does, or the geometry
        // in the other tests is being measured around a different caster.
        let centre = to_pixel.apply([0.0, 0.0]);
        assert!(
            (centre[0] - CASTER_X).abs() <= 0.01 && (centre[1] - AXIS).abs() <= 0.01,
            "{name}: declared at pixel ({:.2}, {:.2}), expected ({CASTER_X}, {AXIS})",
            centre[0],
            centre[1],
        );
    }
}

/// **One ray is the point light, on a caster with edges too.** The ladder starts
/// at the disk's centre, so opening the penumbra and asking for one ray must
/// leave the shadow where the collision walk put it. This is the §29 property
/// one shape over: an angular reject that discarded the centre ray, or a wedge
/// that narrowed the disk before the first tap, would move this.
#[test]
fn one_ray_through_a_polygon_is_the_same_ray_as_the_point_light() {
    for (name, caster) in casters() {
        for probe in [NEAR_PROBE, FAR_PROBE] {
            let hard = silhouette(&profile_with(&caster, 0.0, 1, probe), 1);
            let one = silhouette(&profile_with(&caster, DISK, 1, probe), 1);
            let (hw, ow) = (hard.1 - hard.0, one.1 - one.0);
            assert!(
                (ow - hw).abs() <= 0.5,
                "{name} at probe {probe}: one ray from the disk casts a shadow {ow:.2} px wide \
                 where the point light casts {hw:.2} px — the centre tap is that ray, so the two \
                 widths are the same number",
            );
        }
    }
}

/// **More rays never widen a polygon's umbra**, which is the convexity argument
/// doing its work on edges rather than on slabs: a pixel dark at N+1 rays was
/// dark at N, whatever the plane walk does in between. A wedge that quietly kept
/// a ray alive would grow the dark core, and that is this test.
#[test]
fn more_rays_never_widen_a_polygons_umbra() {
    for (name, caster) in casters() {
        for probe in [NEAR_PROBE, FAR_PROBE] {
            let widths: Vec<f32> = (1..=9u32)
                .map(|taps| core_half(&profile_with(&caster, DISK, taps, probe)))
                .collect();
            for (i, pair) in widths.windows(2).enumerate() {
                assert!(
                    pair[1] <= pair[0] + 0.34,
                    "{name} at probe {probe}: the umbra is {:.2} px wide at {} rays and {:.2} px \
                     at {} — a finer sample of one disk can only find rays that escape: {widths:?}",
                    pair[0],
                    i + 1,
                    pair[1],
                    i + 2
                );
            }
            assert!(
                widths[8] < widths[0],
                "{name} at probe {probe}: nine rays leave a {pop} px umbra where one ray left \
                 {first} px, and the ladder is nested, so the nine-ray core shadow sits inside \
                 the single ray's",
                pop = widths[8],
                first = widths[0]
            );
        }
    }
}

/// **The total light a caster removes moves a little as rays are added, and
/// stops moving after the first few.** The meter's predecessor claimed this
/// number could not move at all, by Fubini: each ray sweeps a hard shadow as
/// wide as the point light's, only displaced. Measured here it does move — a
/// sixteen-sided disc at probe 40 loses 16.00 rows at one ray and 17.50 at six,
/// across rungs 16.00, 16.50, 16.66, 17.24, 17.20, 17.50, 17.14, 16.88, 17.11 —
/// and the reason is in the premise, not in the renderer. "Only displaced" is true
/// under parallel projection. This is a *perspective* projection: an origin
/// nearer the caster subtends it at a steeper angle and sweeps a wider shadow, by
/// up to fifteen per cent across this disk, and the nine taps are not a quadrature
/// of the disk either — five points at radius zero, four at one radius, four at a
/// corner's √2, which weights the outer ring heavier than the area does. So the
/// total is a property of the sample set as much as of the disk, and only the
/// *direction* of the claim survives: the first ray sees the least of the disk and
/// the rungs settle within a row and a half of one another.
///
/// What does not move, and is the sharper instrument, is the set of rows the
/// geometry forces to full light and to full dark: see
/// [`nothing_the_reach_region_cannot_shade_is_shaded`], which needs no averaging
/// argument at all.
#[test]
fn the_total_light_a_polygon_removes_settles_after_the_first_rungs() {
    for (name, caster) in casters() {
        for probe in [NEAR_PROBE, FAR_PROBE] {
            let rows: Vec<f32> = (1..=9u32)
                .map(|taps| darkness(&profile_with(&caster, DISK, taps, probe)))
                .collect();
            let most = rows.iter().copied().fold(0.0_f32, f32::max);
            let least = rows.iter().copied().fold(f32::INFINITY, f32::min);
            // Rows, not a ratio: this caster's shadow is half as wide as the box
            // scene's, so the same absolute movement is twice the percentage, and
            // a percentage inherited from that file would judge the wrong thing.
            // Two rows is the measured spread plus a row of room, and it is loose
            // on purpose — the point is that the number settles, not where.
            assert!(
                most - least <= 2.0,
                "{name} at probe {probe}: the caster eats {most:.2} rows of light at one rung and \
                 {least:.2} at another — a ray's shadow is as wide as the point light's whatever \
                 the ray's offset, so the total cannot move with the sample count: {rows:?}",
            );
        }
    }
}

/// **The outline stays on the axis, and stays inside the disk's reach.** Two
/// bounds from the geometry, both independent of the tap set: the silhouette is
/// the union of the point shadows of the disk's points, so it is centred on the
/// axis however the disk is sampled; and it cannot reach further than the
/// tangent from the disk's rim to the caster's near edge, which is similar
/// triangles in one line. A wedge that rejected a ray it should have walked
/// narrows the outline; one that walked rays outside the disk widens it.
#[test]
fn a_polygons_outline_stays_on_the_axis_and_inside_the_disks_reach() {
    for (name, caster) in casters() {
        for probe in [NEAR_PROBE, FAR_PROBE] {
            let axis = measured_axis_with(&caster, probe);
            let reach = DISK * 2.0_f32.sqrt();
            let outlines: Vec<(f32, f32)> = (1..=9u32)
                .map(|taps| silhouette(&profile_with(&caster, DISK, taps, probe), taps))
                .collect();
            for (rungs, (top, bottom)) in outlines.iter().enumerate() {
                let off = (top + bottom) / 2.0 - axis;
                assert!(
                    off.abs() <= 2.0,
                    "{name} at probe {probe}, {rungs} rays: the outline spans {top:.2}..{bottom:.2}, \
                     centred {off:+.2} px off the axis a point light put at {axis:.2} — the samples \
                     are placed about the light's own centre, so the union of their shadows is \
                     symmetric whatever the rung",
                );
            }
            let widest = outlines
                .iter()
                .map(|(top, bottom)| (axis - top).max(*bottom - axis))
                .fold(0.0_f32, f32::max);

            // Similar triangles, at the near edge of the caster: a hexagon of
            // this radius reaches RADIUS along the axis as well as across it.
            let near = CASTER_X - RADIUS;
            let u = (near - LAMP[0]) / (probe as f32 - LAMP[0]);
            let limit = (RADIUS + reach * (1.0 - u)) / u;
            assert!(
                widest / 2.0 <= limit + 1.0,
                "{name} at probe {probe}: the outline reaches {widest:.2} px from the axis, and no \
                 ray starting inside a {DISK} px disk and clipping a {RADIUS} px caster {near:.0} px \
                 from the lamp can go further than {limit:.2} px",
            );
        }
    }
}

/// FNV-1a over 64 bits: four multiply-free rounds' worth of mixing, no
/// dependency, and long enough that a single flipped byte in 8 192 cannot hide.
/// The fingerprints in §22 and §23 are sixteen digits, so this reproduces their
/// shape as well as their job.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// The bytes a scene is judged on: the target lit on its own, then the same
/// frame with the caster in it. The first half of the pair is the light's own
/// falloff, so a change to the lighting that leaves the shadow alone still moves
/// the fingerprint — which is what "bit-for-bit the same picture" was claiming,
/// rather than only "the same shadow".
fn fingerprint(caster: &Caster, penumbra: f32, taps: u32) -> u64 {
    let mut both = plane(caster, penumbra, taps, false);
    both.extend(plane(caster, penumbra, taps, true));
    fnv1a64(&both)
}

/// A hexagon, freshly built: `Convex` owns its points and the scene list holds
/// each caster by value, so the two rungs of the same shape get their own copy.
fn regular6() -> Caster {
    Caster::Polygon(regular(6, RADIUS))
}

/// The three scenes the fingerprint guard was written with, in the same order:
/// hexagons at nine taps, hexagons at four, circles at nine. The first two are
/// §22's and §23's own; the third is the scene §34 left behind it, once a circle
/// stopped being traced. The numbers in
/// [`the_polygon_fingerprints_print_themselves`] are what those three are on this
/// machine today, which is the claim the document could make but no longer
/// support on its own.
fn scenes() -> Vec<(&'static str, Caster, f32, u32, u64)> {
    let disc = Caster::Circle(RADIUS);
    vec![
        // Taken on this machine on the day the guard was written, from the
        // corrected meter's scene — the geometry tests above all pass with them.
        //
        // The disc's number moved once, when §34 put a circle record under it,
        // and the reading that let it move is there: neither hexagon budged, the
        // disc's outline is byte-identical, and what moved was the light it eats
        // — 24.67 rows to 26.22 at the far probe, against the sixteen-sided coin
        // holding 97.4 % of the circle's area it stands in for. A shape corrected
        // towards what it claims to be should remove a little more light, and it
        // removed a little more light.
        //
        // It moved a second time for §36, and this time the reason is on the record
        // rather than in the reading: the nine-tap road asked whether a pixel stood on
        // the occluder by testing the square drawn around the disc instead of the disc,
        // so four pockets on the diagonals were handed their direct light back. The
        // hexagons did not budge again — a hexagon's planes were always consulted —
        // and neither did the disc's outline, because a pocket is not on an axis and no
        // profile in this file reads a diagonal. That region is what
        // [`super::shadows::both_roads_shadow_the_pocket_between_a_disc_and_its_bounds`]
        // reads now, and it is why this number had to move rather than be argued about:
        // 49 shadowed pocket pixels were lit at radius 14 and 100 at radius 40, and a
        // fingerprint that pins a wrong frame is only a receipt for it.
        (
            "hexagon, nine rays",
            regular6(),
            DISK,
            9,
            0x3cd0_d60c_58d7_9dce,
        ),
        (
            "hexagon, four rays",
            regular6(),
            DISK,
            4,
            0x33a2_a3a4_bbf0_90c5,
        ),
        ("disc, nine rays", disc, DISK, 9, 0x0781_7531_400c_416f),
    ]
}

/// The guard. A byte anywhere in either half of either scene's target changes
/// the number, so an optimisation that claims to preserve the picture has to
/// say so to this test.
///
/// When it fails, the first question is which it is: the geometry tests passing
/// means the picture changed in a way no property of it notices, which is either
/// a legitimate renderer change or a bug too small to see; the geometry tests
/// failing means the renderer is wrong, and the fingerprint is doing you a
/// favour. Re-pin only by running
/// `cargo test --lib the_polygon_fingerprints_print_themselves -- --ignored --nocapture`,
/// reading the profiles it prints, and changing the constants on purpose.
#[test]
fn the_polygon_scenes_fingerprint_what_they_always_did() {
    for (name, caster, penumbra, taps, pinned) in scenes() {
        let now = fingerprint(&caster, penumbra, taps);
        assert_eq!(
            now, pinned,
            "{name} at {taps} rays: the frame is {:016x} and the pinned fingerprint is \
             {pinned:016x}. If the renderer was meant to change, re-pin deliberately.",
            now,
        );
    }
}

/// The printer, and the reading that has to happen before anything is re-pinned.
#[test]
#[ignore = "prints; run it to re-pin the fingerprints above"]
fn the_polygon_fingerprints_print_themselves() {
    for (name, caster, penumbra, taps, pinned) in scenes() {
        let now = fingerprint(&caster, penumbra, taps);
        println!("{name:18} at {taps} rays: {now:016x}  (pinned {pinned:016x})");
        for probe in [NEAR_PROBE, FAR_PROBE] {
            let p = profile_with(&caster, penumbra, taps, probe);
            let (t, b) = silhouette(&p, taps);
            println!(
                "    probe {probe}: outline {t:.2}..{b:.2} (half {:.2}) | umbra half {:.2} | \
                 darkness {:.2} rows",
                (b - t) / 2.0,
                core_half(&p),
                darkness(&p),
            );
        }
    }
}

/// **Nothing outside the reach region is shaded, and everything inside the
/// whole-disk shadow is.** The rotation-free CPU oracle: for every row it asks
/// all ninety-seven origins in the tap region whether the segment meets the
/// caster, and asserts only where the answer is unanimous. A row no origin can
/// shade must read lit — that is where an over-wide reject, a stale bounding box,
/// or a wedge that dropped a ray shows up. A row every origin shades must read
/// dark — that is where an under-blocking reject, a plane list that lost an
/// edge, or a t-interval that lets a ray through shows up.
///
/// It holds for any tap count, any order, and any per-pixel rotation, which is
/// the reason it exists: this is the assertion a per-pixel CPU twin could not
/// make, and the reason §20's twin was abandoned.
#[test]
fn nothing_the_reach_region_cannot_shade_is_shaded() {
    for (name, caster) in casters() {
        for probe in [NEAR_PROBE, FAR_PROBE] {
            for taps in 1..=9u32 {
                let (never, always) = verdict(&caster, probe, reach_of(taps, DISK));
                let p = profile_with(&caster, DISK, taps, probe);
                // The outermost and innermost row of each verdict run is left
                // alone: a row grazing the caster's edge is where the CPU's
                // endpoint convention and the slab clip's may legitimately
                // disagree by one sample, and neither direction of this test is
                // about a single grazing row.
                for y in never.iter().skip(1).rev().skip(1).rev() {
                    assert!(
                        p[*y as usize] > 0.98,
                        "{name} at probe {probe}, {taps} rays: row {y} reads {:.2} of its light \
                         although no origin in the tap region reaches it",
                        p[*y as usize],
                    );
                }
                for y in always.iter().skip(1).rev().skip(1).rev() {
                    assert!(
                        p[*y as usize] < 0.02,
                        "{name} at probe {probe}, {taps} rays: row {y} reads {:.2} of its light \
                         although every origin in the tap region is behind the caster",
                        p[*y as usize],
                    );
                }
            }
        }
    }
}

/// The rows both sides say are dark, for the point-sized case: the readback's
/// and the collision walk's, as runs. Written to answer one question — whether
/// the two disagree about where the shadow is, or only about the row convention
/// that puts a world coordinate between two pixel centres.
#[test]
#[ignore = "prints the two dark runs side by side"]
fn the_polygon_hard_shadow_shows_its_rows() {
    for (name, caster) in casters() {
        let (_, to_local) = declared(&caster);
        let from_local = to_local.apply(LAMP);
        let (cpu_top, cpu_bottom) = cpu_shadow(from_local, &to_local, &caster);
        let red = plane(&caster, 0.0, 1, true);
        let rows: Vec<usize> = (0..SPAN as usize)
            .filter(|y| red[(y * SPAN as usize) + NEAR_PROBE as usize] < 128)
            .collect();
        let p = profile_with(&caster, 0.0, 1, NEAR_PROBE);
        println!(
            "{name}: cpu {cpu_top:.2}..{cpu_bottom:.2} | shader {}..{} ({} rows)",
            rows.first().copied().unwrap_or(0),
            rows.last().copied().unwrap_or(0),
            rows.len(),
        );
        println!("    profile 21..26: {:?}", &p[21..26]);
        println!("    profile 37..42: {:?}", &p[37..42]);
        println!("    silhouette() says {:?}", silhouette(&p, 1));
    }
}
