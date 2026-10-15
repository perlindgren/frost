//! Bloom is light arriving where the shape that emitted it is not. That is the
//! whole contract, and it is the one thing every previous test in this tree
//! could not check: the emission tests look at the pixel *inside* a plate, and
//! bloom by definition moves light to the pixels outside it. So the assertions
//! below are about the pixels beside the lamp, painted by the shared present
//! harness in `present.rs`.

use super::present::{MIDDLE, SPAN, frame, lamp, lamp_at};
use crate::backend::frame::Draw;
use crate::objects;
use crate::objects::Transform;

/// Light arrives where the lamp is not, and thins out as it goes.
///
/// Three strengths of the same frame: no bloom, half, and full. The plate's
/// own pixels barely matter here — what is checked is a pixel eight places
/// outside its edge, a pixel twenty-four outside, and the difference between
/// the two, which is the whole of what a glow is.
#[test]
fn bloom_lands_beside_the_lamp_and_thins_as_it_travels() {
    let Some(quiet) = frame(&[lamp(4.0)], 0.0, 0.0, None, None) else {
        return;
    };
    let Some(half) = frame(&[lamp(4.0)], 0.5, 0.0, None, None) else {
        return;
    };
    let Some(full) = frame(&[lamp(4.0)], 1.0, 0.0, None, None) else {
        return;
    };
    let at = |x: u32, y: u32| usize::try_from(y * SPAN + x).expect("in frame");
    // The plate spans 26..38; these are outside it, to the right, on its
    // middle row. `near` is eight pixels past the plate's edge, `far` twenty-one.
    let near = at(46, MIDDLE as u32);
    let far = at(62, MIDDLE as u32);
    let sum = |p: &[f32; 3]| p[0] + p[1] + p[2];

    // The pixel beside the lamp held nothing at all without bloom — the frame
    // is a black background with one plate on it, so any light there is light
    // the chain moved.
    assert!(
        sum(&quiet[near]) < 1e-3,
        "the pixel outside the plate held {:?} with bloom off, and the frame \
         has nothing else in it",
        quiet[near]
    );
    let glow_near = sum(&full[near]) - sum(&quiet[near]);
    let glow_far = sum(&full[far]) - sum(&quiet[far]);
    let glow_half = sum(&half[near]) - sum(&quiet[near]);
    // Measured here on an M4 Max: 0.0124 five pixels out, 0.0015 twenty-one
    // out. The numbers are small because this is a 12-pixel plate in a 64-pixel
    // frame and a glow spreads one plate's light over an area hundreds of times
    // its size; the assertions sit at a third of what was measured, which leaves
    // room for another GPU's filtering without letting a chain that produced
    // nothing pass.
    assert!(
        glow_near > 0.004,
        "a plate putting out 1.6 of radiance left only {glow_near} of light \
         five pixels away; bloom is not arriving"
    );
    // It thins on the way: a glow that is uniform across the frame is not a
    // glow, it is a haze over the whole picture, and that is what a chain
    // whose levels are weighted wrong turns into.
    assert!(
        glow_near > glow_far * 3.0,
        "the glow does not fall off: {glow_near} eight pixels out against \
         {glow_far} twenty-one out (eight to one here)"
    );
    // And the knob is linear in what it adds: half the strength, half the glow
    // (measured 0.0062 against 0.0124 — exactly half, because the weights are
    // multiplied by strength on the CPU and nothing clamps on the way).
    assert!(
        glow_half > glow_near * 0.4 && glow_half < glow_near * 0.6,
        "at half strength the glow should be about half: {glow_half} against \
         {glow_near}"
    );
}

/// Nothing emissive means nothing to bloom: the same frame with the same
/// strength comes back exactly as it went in.
///
/// This is the assertion that keeps the threshold honest. Bloom that adds
/// light to a scene with no emission is not a glow, it is a brightness change,
/// and the difference only shows up as a scene that got washed out for no
/// reason anyone can point at.
#[test]
fn a_frame_that_emits_nothing_gains_nothing() {
    let Some(quiet) = frame(&[lamp(0.0)], 0.0, 0.0, None, None) else {
        return;
    };
    let Some(bloomed) = frame(&[lamp(0.0)], 1.0, 0.0, None, None) else {
        return;
    };
    let worst = quiet.iter().zip(&bloomed).fold(0.0f32, |m, (a, b)| {
        m.max(
            (a[0] - b[0])
                .abs()
                .max((a[1] - b[1]).abs())
                .max((a[2] - b[2]).abs()),
        )
    });
    assert_eq!(
        worst, 0.0,
        "a frame with no emission changed by {worst} at its worst pixel"
    );
    // And the plate is still there, so the equality above is not two black
    // frames agreeing with each other.
    let centre = usize::try_from(MIDDLE as u32 * SPAN + MIDDLE as u32).expect("in frame");
    assert!(
        bloomed[centre][0] > 0.3,
        "the plate itself vanished: {:?}",
        bloomed[centre]
    );
}

/// Writes two pictures to look at, for whoever is judging the look rather than
/// the arithmetic. Ignored, because a test that writes files is not a test.
#[test]
#[ignore = "writes /tmp/frost_bloom_*.ppm for a human to look at"]
fn paint_two_pictures_to_look_at() {
    frame(
        &[lamp(2.0)],
        0.0,
        0.0,
        Some("/tmp/frost_bloom_off.ppm"),
        None,
    );
    frame(
        &[lamp(2.0)],
        1.0,
        0.0,
        Some("/tmp/frost_bloom_on.ppm"),
        None,
    );
    // Three lamps of different authored strength over a dim floor: the case a
    // look is actually judged on, where the threshold's ramp should show as one
    // lamp glowing hard, one softly, and one barely at all.
    let scene = [
        // A wall to stand behind them, unlit and dim.
        Draw::Shape {
            world: Transform::identity(),
            center: [MIDDLE, 50.0],
            params: [30.0, 8.0],
            kind: 1.0,
            aa: 1.0,
            color: objects::Color {
                r: 0.08,
                g: 0.09,
                b: 0.12,
                a: 1.0,
            },
            emission: objects::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            glow: None,
            lit: 0.0,
            occludes: 0.0,
            z: 0.0,
            diagnostic: false,
        },
        lamp_at([14.0, 20.0], 0.4),
        lamp_at([32.0, 20.0], 1.5),
        lamp_at([50.0, 20.0], 4.0),
    ];
    frame(&scene, 0.0, 0.0, Some("/tmp/frost_scene_off.ppm"), None);
    frame(&scene, 1.0, 0.0, Some("/tmp/frost_scene_on.ppm"), None);
}

/// The two colors of a node stay separate: `SceneNode::glow` is the color of
/// the light as the chain sees it, and the surface keeps its own emission.
///
/// A plate that emits amber is asked to glow cold. Two things have to be true
/// at once, and they are the whole reason the second field exists: with the
/// chain off the picture is byte for byte the picture the emission alone paints
/// — the field is never read by the color the eye lands on — and with the chain
/// on, the light beside the plate changes hue. A field that leaked into the
/// frame, or did nothing at all, fails one of the two.
#[test]
fn a_split_glow_colours_the_halo_and_not_the_surface() {
    let amber = objects::Color {
        r: 1.0,
        g: 0.55,
        b: 0.15,
        a: 2.0,
    };
    let cold = objects::Color {
        r: 0.2,
        g: 0.5,
        b: 2.0,
        a: 2.0,
    };
    let plate = |glow: Option<objects::Color>| Draw::Shape {
        world: Transform::identity(),
        center: [MIDDLE, MIDDLE],
        params: [6.0, 6.0],
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 0.9,
            g: 0.9,
            b: 0.9,
            a: 1.0,
        },
        emission: amber,
        glow,
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
        diagnostic: false,
    };
    let at = |x: u32, y: u32| usize::try_from(y * SPAN + x).expect("in frame");
    let (own, beside) = (at(32, 32), at(46, 32));

    let warm_flat = frame(&[plate(None)], 0.0, 0.0, None, None).expect("a frame");
    let cold_flat = frame(&[plate(Some(cold))], 0.0, 0.0, None, None).expect("a frame");
    assert_eq!(
        warm_flat[own], cold_flat[own],
        "the glow field reached the surface: a split glow is allowed to change \
         only the light the chain carries"
    );

    let warm = frame(&[plate(None)], 1.0, 0.0, None, None).expect("a frame");
    let chilly = frame(&[plate(Some(cold))], 1.0, 0.0, None, None).expect("a frame");
    // Both frames must actually glow, or the hue comparison below is vacuous.
    let glow = |p: &[[f32; 3]]| p[beside][0] + p[beside][1] + p[beside][2];
    assert!(
        glow(&warm) > 1e-3 && glow(&chilly) > 1e-3,
        "neither frame glowed: {:.5} and {:.5}",
        glow(&warm),
        glow(&chilly)
    );
    // The amber glow is reddest; the cold one bluest. The ratio, not the level:
    // the two are free to differ in brightness, since the channels asked for
    // different amounts of light.
    let hue = |p: &[[f32; 3]]| (p[beside][2] - p[beside][0]) / glow(p);
    assert!(
        hue(&warm) < -0.1 && hue(&chilly) > 0.1,
        "the halo kept the surface's hue: amber {:?}, cold {:?}, beside-pixel \
         {:?} against {:?}",
        hue(&warm),
        hue(&chilly),
        warm[beside],
        chilly[beside]
    );
}
