//! Text expansion: glyph sprites are spliced into the scene in place, off one shared atlas.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;
use crate::text;
use std::collections::HashMap;
use std::sync::Arc;

/// The font file used by the expand_text tests, loaded from the crate's
/// asset directory.
fn test_font() -> Arc<[u8]> {
    Arc::from(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/JameGem08_2026-Regular.ttf"
        ))
        .expect("test font should be readable"),
    )
}

#[test]
fn expand_text_splices_glyph_sprites_in_place() {
    // A text draw between two circle draws must be replaced by its
    // glyph sprites without disturbing the neighbours' positions: the
    // circles keep their slots and the sprites inherit the text's tint,
    // alpha and z.
    let font = test_font();
    let size = 48.0;
    let mut canvas = Canvas::new((800, 600));
    let tint = Color {
        r: 1.0,
        g: 0.5,
        b: 0.25,
        a: 1.0,
    };
    canvas.draws = vec![
        Draw::Circle {
            diagnostic: false,
            center: [-100.0, 0.0],
            radius: 10.0,
            color: black(),
            z: 0.0,
        },
        Draw::Text {
            diagnostic: false,
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: "hi".to_string(),
            size,
            weight: 400.0,
            color: tint,
            alpha: 0.9,
            lit: 1.0,
            z: 1.0,
        },
        Draw::Circle {
            diagnostic: false,
            center: [100.0, 0.0],
            radius: 10.0,
            color: black(),
            z: 2.0,
        },
    ];
    let mut atlases: HashMap<(u64, u32, u32), text::Atlas> = HashMap::new();
    canvas.expand_text(&mut atlases);

    // The layout's glyph count is the source of truth for how many
    // sprites the text should produce (every glyph of "hi" has ink).
    let layout = text::layout(&font, "hi", size, 400.0).expect("the test font should shape 'hi'");
    assert_eq!(canvas.draws.len(), 2 + layout.glyphs.len());
    let [Draw::Circle { z: z0, .. }, .., Draw::Circle { z: z1, .. }] = &canvas.draws[..] else {
        panic!("the circles must keep their slots");
    };
    assert_eq!(*z0, 0.0);
    assert_eq!(*z1, 2.0);
    for draw in canvas.draws.iter().skip(1).take(layout.glyphs.len()) {
        let Draw::Sprite {
            size,
            texture_size,
            uv_rect,
            tint,
            alpha,
            lit,
            z,
            ..
        } = draw
        else {
            panic!("every spliced draw must be a sprite");
        };
        assert_eq!(
            *tint,
            Color {
                r: 1.0,
                g: 0.5,
                b: 0.25,
                a: 1.0
            }
        );
        assert_eq!(*alpha, 0.9);
        // The block's `lit` flag passes on to every glyph quad.
        assert_eq!(*lit, 1.0);
        assert_eq!(*z, 1.0);
        // Glyph quads are a sub-rectangle of the 512x512 atlas.
        assert_eq!(*texture_size, [512, 512]);
        assert!(size[0] > 0.0 && size[1] > 0.0);
        assert!(uv_rect[0] < uv_rect[2]);
        assert!(uv_rect[1] < uv_rect[3]);
    }
}

#[test]
fn expand_text_reuses_the_atlas_across_frames() {
    // A second frame with the same font, text and size must not
    // re-rasterize: the atlas data Arc stays the same pointer, so the
    // GPU texture is reused and no bytes are re-uploaded.
    let font = test_font();
    let size: f32 = 48.0;
    let mut atlases: HashMap<(u64, u32, u32), text::Atlas> = HashMap::new();
    let key = (
        Arc::as_ptr(&font) as *const () as u64,
        size.to_bits(),
        400.0f32.to_bits(),
    );

    let mut frame = || {
        let mut canvas = Canvas::new((800, 600));
        canvas.draws = vec![Draw::Text {
            diagnostic: false,
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: "hello world".to_string(),
            size,
            weight: 400.0,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
            lit: 0.0,
            z: 0.0,
        }];
        canvas.expand_text(&mut atlases);
        let Draw::Sprite { data, .. } = &canvas.draws[0] else {
            panic!("the first glyph must be a sprite");
        };
        Arc::as_ptr(data)
    };
    let first = frame();
    let second = frame();
    assert_eq!(first, second, "frame two must reuse the atlas buffer");
    // The atlas was actually registered under its font key.
    assert!(atlases.contains_key(&key));
}

#[test]
fn expand_text_packs_glyphs_first_seen_on_a_later_frame() {
    // The atlas key (font, size, weight) is registered on the first frame; a later
    // frame whose text introduces new glyphs must rasterize them into the
    // same persistent atlas and draw them. The diagnostics readout relies
    // on this: its "FPS" line's glyphs never appear in its "size" line's
    // first-frame text.
    let font = test_font();
    let size = 48.0;
    let mut atlases: HashMap<(u64, u32, u32), text::Atlas> = HashMap::new();

    let mut frame = |text: &str| -> usize {
        let mut canvas = Canvas::new((800, 600));
        canvas.draws = vec![Draw::Text {
            diagnostic: false,
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: text.to_string(),
            size,
            weight: 400.0,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
            lit: 0.0,
            z: 0.0,
        }];
        canvas.expand_text(&mut atlases);
        canvas.draws.len()
    };

    // The first frame seeds the atlas with its seven inked glyphs.
    assert_eq!(frame("800x600"), 7);
    // The second frame's "F", "P", "S" and "5" have never been packed
    // before: all five inked glyphs must be drawn ("8" was packed by the
    // first text; the space has no ink and gets no quad).
    assert_eq!(
        frame("FPS 58"),
        5,
        "glyphs first seen on a later frame must be packed and drawn"
    );
}
