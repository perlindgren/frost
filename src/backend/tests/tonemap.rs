//! What a value past white turns into. That is the whole job of a tone map,
//! and the interesting part is that the frame buffer is not where it fails:
//! `Rgba16Float` holds 4.0 quite happily, and so does the harness's window. The
//! failure happens one stage later, at a surface that cannot show more than
//! 1.0, where everything above it becomes the same white. So every assertion
//! here passes the readback through what an eight-bit surface would actually
//! have received — the transfer curve, then the clamp, because it has no room
//! for anything past 1.0 — and asks the question in the display's own terms.
//!
//! Two things are worth pinning, and they are the two a tone map can get wrong
//! in opposite directions. Light above white has to arrive distinguishable and
//! in its own colour; and light that was never above white must not move at
//! all, because a curve that reshapes the ordinary part of the range is not a
//! tone map, it is a filter over the whole game.

use super::present::{MIDDLE, SPAN, frame, lamp, lamp_at};

/// The plate's own pixel, at the frame's centre.
fn centre() -> usize {
    usize::try_from(MIDDLE as u32 * SPAN + MIDDLE as u32).expect("in frame")
}

/// What an eight-bit unorm surface would hold at this pixel: the transfer curve
/// the surface applies on write, and the clamp afterwards.
fn on_a_display(c: &[f32; 3]) -> [f32; 3] {
    let encode = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    [encode(c[0]), encode(c[1]), encode(c[2])]
}

/// Below the knee the curve is the identity, so a frame with nothing bright in
/// it has to come back *exactly* as it went in — not close, not within a
/// tolerance. That is what "an ordinary frame is not touched" means, and it is
/// the assertion a curve that quietly lifts or dims the mid-tones fails.
#[test]
fn light_that_never_goes_past_white_does_not_move() {
    // A plate at half its own colour: 0.6 of radiance, against a knee at 0.85.
    let Some(clip) = frame(&[lamp(0.5)], 0.0, 0.0, None, None) else {
        return;
    };
    let Some(folded) = frame(&[lamp(0.5)], 0.0, 0.15, None, None) else {
        return;
    };
    let worst = clip.iter().zip(&folded).fold(0.0f32, |m, (a, b)| {
        m.max(
            (a[0] - b[0])
                .abs()
                .max((a[1] - b[1]).abs())
                .max((a[2] - b[2]).abs()),
        )
    });
    assert_eq!(
        worst, 0.0,
        "a frame whose brightest pixel is 0.6 moved by {worst} when the knee was 0.85"
    );
    // And there was a plate in it, so the equality above is not two black
    // frames agreeing with each other.
    assert!(
        folded[centre()][0] > 0.4,
        "the plate vanished: {:?}",
        folded[centre()]
    );
}

/// The reason to have a tone map at all: a lamp brighter than the display is
/// not one colour, it is a lamp, and it is still that lamp whether it puts out
/// twice white or ten times.
#[test]
fn light_above_white_arrives_as_a_colour_and_not_as_white() {
    // The plate's authored colour is (0.4, 0.2, 0.1), so at emission 4.0 it
    // puts out (1.6, 0.8, 0.4): above white in every channel, and amber.
    let Some(clip) = frame(&[lamp(4.0)], 0.0, 0.0, None, None) else {
        return;
    };
    let Some(folded) = frame(&[lamp(4.0)], 0.0, 0.15, None, None) else {
        return;
    };
    let before = on_a_display(&clip[centre()]);
    let after = on_a_display(&folded[centre()]);
    // What clipping actually does, stated as the test's premise rather than as
    // a guess: the two channels above white are pinned to the ceiling, and the
    // third is left behind at its own value. The lamp's colour was in the
    // *ratio* between them, and the ratio is what the ceiling takes away —
    // 2.0 becomes 1.0, amber becomes pale yellow, and a lamp at twice this
    // brightness would arrive exactly the same.
    assert!(
        before[0] > 0.999 && before[1] > 0.999 && before[2] < 0.9,
        "clipped, the plate should have had two channels pinned and one left; \
         it came back {before:?}"
    );
    // Folded, nothing reaches the ceiling — in the frame's own linear numbers,
    // where the ceiling is — so the ratios the artist wrote are still in the
    // pixel. The shader scales a pixel by the ratio its brightest channel maps
    // at, which preserves hue by construction, and this is where the
    // construction gets checked.
    let at = &folded[centre()];
    assert!(
        at[0] < 1.0,
        "folded, the plate still reached the ceiling: {at:?}"
    );
    // On the screen it is a lamp again rather than a white one: the channel
    // clipping left behind is nowhere near the ceiling once the fold has run.
    assert!(
        after[2] < 0.75,
        "folded, the lamp still reads as white on a display: {after:?}"
    );
    let (r, g) = (at[0] / at[1], at[1] / at[2]);
    assert!(
        (r - 2.0).abs() < 0.02 && (g - 2.0).abs() < 0.02,
        "the fold drained the colour out of it: {r:.3} and {g:.3} against 2.0 and 2.0"
    );
}

/// A rational curve approaches white slowly on purpose. This is the measurement
/// of why: lamps a factor of two apart have to stay apart on the screen all the
/// way up the range, where an exponential shoulder would have merged them into
/// one pixel.
#[test]
fn brighter_stays_brighter_all_the_way_up() {
    let mut seen: Vec<f32> = vec![];
    for emission in [0.5f32, 1.0, 2.0, 4.0, 8.0, 16.0] {
        let Some(painted) = frame(&[lamp(emission)], 0.0, 0.15, None, None) else {
            return;
        };
        let on = on_a_display(&painted[centre()]);
        // Nothing escapes the display's range, which is the other half of the
        // contract: a fold that still overflows has bought nothing.
        assert!(
            on.iter().all(|v| *v <= 1.0),
            "emission {emission} left the range: {on:?}"
        );
        seen.push(on[0]);
    }
    for pair in seen.windows(2) {
        assert!(
            pair[1] > pair[0] + 0.001,
            "two lamps a factor of two apart came back the same: {pair:?}"
        );
    }
    // The last is the one worth looking at: sixteen times the authored colour
    // is still short of white, which is what the headroom is for.
    assert!(
        *seen.last().expect("at least one lamp") < 1.0,
        "the brightest lamp saturated anyway: {seen:?}"
    );
}

/// Bloom and the tone map are asked for separately, and the second must not
/// cost the first anything.
///
/// The interesting version of this is not the obvious one. The fold is applied
/// after the bloom is added, which is the right order — the total is what has to
/// fit the display — and the interesting consequence is what it means *for the
/// halo*: a pixel the glow reached is far below the knee, so the fold is the
/// identity there and the glow goes through to the screen untouched. Asking
/// whether the fold bites on the halo turns out to be unanswerable in the other
/// direction by construction: a surface is emissive enough to bloom only if it
/// is already brighter than any knee worth having, so there is no frame whose
/// own pixels are dim enough to skip the fold and bright enough to throw a
/// glow. What can be pinned, and is worth pinning, is that the halo survives.
#[test]
fn the_fold_lets_the_halo_through() {
    let at = |x: u32, y: u32| usize::try_from(y * SPAN + x).expect("in frame");
    // Eight pixels outside the plate, on its middle row.
    let beside = at(46, MIDDLE as u32);
    let Some(light) = frame(&[lamp(4.0)], 1.0, 0.0, None, None) else {
        return;
    };
    let Some(folded) = frame(&[lamp(4.0)], 1.0, 0.15, None, None) else {
        return;
    };
    let sum = |p: &[f32; 3]| p[0] + p[1] + p[2];
    assert!(
        sum(&light[beside]) > 0.004,
        "there was no halo there at all: {:.5}",
        sum(&light[beside])
    );
    assert_eq!(
        light[beside], folded[beside],
        "the fold moved a pixel the glow reached: {:?} against {:?}",
        light[beside], folded[beside]
    );
    // And on the plate, where the total is past white, the fold does bite —
    // otherwise the assertion above would only be showing that nothing runs.
    let centre = at(MIDDLE as u32, MIDDLE as u32);
    assert!(
        sum(&folded[centre]) < sum(&light[centre]),
        "the fold did nothing anywhere: {:.4} against {:.4}",
        sum(&folded[centre]),
        sum(&light[centre])
    );
}

/// The pictures, for whoever is judging the look rather than the arithmetic:
/// one lamp, with the headroom at none, at a sixth, and at a third. Ignored,
/// because a test that writes files is not a test.
#[test]
#[ignore = "writes /tmp/frost_tone_*.ppm for a human to look at"]
fn paint_the_three_heads() {
    for (name, headroom) in [("none", 0.0f32), ("small", 0.06), ("big", 0.33)] {
        let which = match name {
            "none" => "/tmp/frost_tone_none.ppm",
            "small" => "/tmp/frost_tone_small.ppm",
            _ => "/tmp/frost_tone_big.ppm",
        };
        frame(&[lamp(4.0)], 0.5, headroom, Some(which), None);
    }
    // And the same three lamps a factor of four apart, once clipped and once
    // folded: the pair a decision about the headroom is actually made on.
    let trio = [
        lamp_at([14.0, 32.0], 0.5),
        lamp_at([32.0, 32.0], 2.0),
        lamp_at([50.0, 32.0], 8.0),
    ];
    frame(&trio, 0.5, 0.0, Some("/tmp/frost_tone_trio_clip.ppm"), None);
    frame(
        &trio,
        0.5,
        0.15,
        Some("/tmp/frost_tone_trio_fold.ppm"),
        None,
    );
}
