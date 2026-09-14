//! Text layout and glyph atlas, on the CPU side.
//!
//! A [`Shape::Text`] node is drawn with texture quads: the text is shaped
//! into positioned glyphs, every unique glyph is rasterized once per
//! (font, size) pair and packed into a shared shelf [`Atlas`], and each
//! placed glyph becomes a sprite quad sampling its cell in the atlas
//! texture. This module is pure CPU code; the GPU side is the regular
//! sprite pipeline.
//!
//! All coordinates here are in font pixels at the layout size, with y up
//! (the scene's convention), and glyph offsets relative to the text's pen
//! origin — the left end of the baseline.

use std::collections::HashMap;
use std::sync::Arc;

use swash::scale::{Render, ScaleContext, Source};
use swash::shape::ShapeContext;
use swash::FontRef;

/// The fixed atlas size in pixels; a glyph that does not fit is skipped.
const ATLAS_SIZE: u32 = 512;

/// A text laid out at a size in pixels: the pen positions of every glyph
/// plus the block's overall metrics.
pub(crate) struct TextLayout {
    /// Distance from the baseline to the top of the text.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the text.
    pub descent: f32,
    /// The total advance width of the text.
    pub width: f32,
    /// Every shaped glyph, in reading order.
    pub glyphs: Vec<PlacedGlyph>,
}

/// One shaped glyph and its pen position, relative to the text's pen
/// origin (the left end of the baseline), y up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedGlyph {
    /// The glyph's id in the font.
    pub id: u16,
    /// The glyph's pen x offset.
    pub x: f32,
    /// The glyph's pen y offset (zero for ordinary text; non-zero for
    /// subscripts and superscripts).
    pub y: f32,
}

/// Shapes `text` with `font` at `size` pixels per em and returns the pen
/// position of every glyph, or `None` if `font` is not a usable
/// TrueType/OpenType font.
pub(crate) fn layout(font: &[u8], text: &str, size: f32) -> Option<TextLayout> {
    let font = FontRef::from_index(font, 0)?;
    let mut context = ShapeContext::new();
    let mut shaper = context.builder(font).size(size).build();
    shaper.add_str(text);
    let metrics = shaper.metrics().scale(size);
    let mut width = 0.0;
    let mut glyphs = Vec::new();
    shaper.shape_with(|cluster| {
        // `glyph.x` is the offset from this cluster's pen, so the pen's
        // position must be added on.
        let cluster_pen = width;
        width += cluster.advance();
        for glyph in cluster.glyphs {
            glyphs.push(PlacedGlyph {
                id: glyph.id,
                x: cluster_pen + glyph.x,
                y: glyph.y,
            });
        }
    });
    Some(TextLayout {
        ascent: metrics.ascent,
        descent: metrics.descent,
        width,
        glyphs,
    })
}

/// A rasterized glyph mask, ready to be packed into an [`Atlas`].
#[derive(Clone)]
pub(crate) struct RasterGlyph {
    /// The mask as RGBA8, rows top first, the 8-bit alpha replicated into
    /// every channel (so the sprite shader's tint/alpha blending works on
    /// a solid color).
    pub data: Vec<u8>,
    /// The mask's width in pixels.
    pub width: u32,
    /// The mask's height in pixels.
    pub height: u32,
    /// The pen-space x of the mask's left edge, y up.
    pub left: i32,
    /// The pen-space y of the mask's top edge, y up.
    pub top: i32,
}

/// Rasterizes the font's glyph `id` at `size` pixels per em into an 8-bit
/// alpha mask, or `None` if the font is unusable or the glyph has no
/// outlines (a space, for example).
pub(crate) fn rasterize(font: &[u8], id: u16, size: f32) -> Option<RasterGlyph> {
    let font = FontRef::from_index(font, 0)?;
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(size).build();
    let render = Render::new(&[Source::Outline]);
    let image = render.render(&mut scaler, id)?;
    let width = image.placement.width;
    let height = image.placement.height;
    if width == 0 || height == 0 {
        return None;
    }
    // swash renders with a BottomLeft origin, but the mask buffer is still
    // written top first (row 0 of `image.data` is the glyph's *top* row)
    // and `placement.top` is the pen-space y of that top row, so the rows
    // copy straight across. Replicate the mask's alpha into RGBA so the
    // sprite shader tints and fades it like any texture.
    let mut data = vec![0u8; (width as usize) * (height as usize) * 4];
    for row in 0..height as usize {
        let src = row * width as usize;
        let dst = row * (width as usize) * 4;
        for col in 0..width as usize {
            let a = image.data[src + col];
            data[dst + col * 4..dst + col * 4 + 4].copy_from_slice(&[a, a, a, a]);
        }
    }
    Some(RasterGlyph {
        data,
        width,
        height,
        left: image.placement.left,
        top: image.placement.top,
    })
}

/// One packed glyph: where its mask sits in the atlas, and where that mask
/// sits in pen space.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    /// The cell's left edge, in atlas pixels.
    pub x: u32,
    /// The cell's top edge, in atlas pixels, rows top first.
    pub y: u32,
    /// The mask's width in pixels.
    pub width: u32,
    /// The mask's height in pixels.
    pub height: u32,
    /// The pen-space x of the mask's left edge, y up.
    pub left: i32,
    /// The pen-space y of the mask's top edge, y up.
    pub top: i32,
}

/// A fixed-size shelf atlas of glyph masks.
///
/// Glyphs are packed left to right along shelves; a glyph that does not
/// fit in the current shelf starts a new one. The atlas never grows, so a
/// glyph that is wider or taller than it — or that would need a shelf
/// below the last one — is simply skipped.
#[derive(Clone)]
pub(crate) struct Atlas {
    /// The atlas pixels, RGBA8, rows top first.
    pub data: Arc<[u8]>,
    /// The atlas's width in pixels.
    pub width: u32,
    /// The atlas's height in pixels.
    pub height: u32,
    cells: HashMap<u16, Cell>,
    /// The x cursor along the current shelf.
    x: u32,
    /// The top of the current shelf, in atlas pixels.
    y: u32,
    /// The height of the current shelf.
    row_height: u32,
}

impl Default for Atlas {
    fn default() -> Self {
        Self {
            data: Arc::from(vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize]),
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            cells: HashMap::new(),
            x: 0,
            y: 0,
            row_height: 0,
        }
    }
}

impl Atlas {
    /// Whether the glyph `id` is already packed in the atlas.
    pub fn has(&self, id: u16) -> bool {
        self.cells.contains_key(&id)
    }

    /// The cell of a packed glyph.
    pub fn cell(&self, id: u16) -> Option<&Cell> {
        self.cells.get(&id)
    }

    /// Packs `glyphs` into the atlas, starting a new shelf when needed.
    /// Glyphs that are already packed, or too big for the atlas, are
    /// skipped; the buffer is only rebuilt when at least one new glyph
    /// lands.
    pub fn insert_many(&mut self, glyphs: &[(u16, RasterGlyph)]) {
        let mut data = self.data.to_vec();
        // The buffer is only re-Arc'd when a glyph actually lands, so an
        // empty or fully-skipped call leaves the existing Arc untouched and
        // the GPU texture reusable.
        let mut changed = false;
        for (id, glyph) in glyphs {
            if self.has(*id) || glyph.width > self.width || glyph.height > self.height {
                continue;
            }
            if self.x + glyph.width > self.width {
                self.x = 0;
                self.y += self.row_height;
                self.row_height = 0;
            }
            if self.y + glyph.height > self.height {
                break; // Out of atlas space; keep what packed above.
            }
            let row_bytes = (glyph.width * 4) as usize;
            for row in 0..glyph.height as usize {
                let src = row * row_bytes;
                let dst = (((self.y + row as u32) * self.width + self.x) as usize) * 4;
                data[dst..dst + row_bytes].copy_from_slice(&glyph.data[src..src + row_bytes]);
            }
            self.cells.insert(
                *id,
                Cell {
                    x: self.x,
                    y: self.y,
                    width: glyph.width,
                    height: glyph.height,
                    left: glyph.left,
                    top: glyph.top,
                },
            );
            self.x += glyph.width;
            self.row_height = self.row_height.max(glyph.height);
            changed = true;
        }
        if changed {
            self.data = Arc::from(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo's example font, used by the layout and raster tests.
    fn example_font() -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/JameGem08_2026-Regular.ttf");
        std::fs::read(path).expect("the example font should ship with the repo")
    }

    /// Shaping "hello world" at 48 px yields one glyph per character, a
    /// forward-moving pen, and positive vertical metrics.
    #[test]
    fn layout_shapes_the_example_text() {
        let font = example_font();
        let layout = layout(&font, "hello world", 48.0)
            .expect("the example font should shape the text");
        assert!(layout.ascent > 0.0, "the ascent should be positive");
        assert!(layout.descent >= 0.0, "the descent cannot be negative");
        // "hello world" is eleven characters; allow for a ligature or two.
        assert!(
            layout.glyphs.len() >= 10,
            "expected at least 10 glyphs, got {}",
            layout.glyphs.len()
        );
        assert!(layout.width > 100.0, "the text should be wider than 100 px");
        let mut x = layout.glyphs[0].x;
        for glyph in &layout.glyphs[1..] {
            assert!(
                glyph.x >= x,
                "the pen should not move backwards: {x} -> {}",
                glyph.x
            );
            x = glyph.x;
        }
        // The pen must actually travel across the text; with a dozen or so
        // glyphs the last one starts well past the middle of the block
        // (this catches cluster-relative offsets that never advance).
        let last = layout.glyphs.last().expect("there should be a last glyph");
        assert!(
            last.x > layout.width / 2.0,
            "the last glyph's pen should be past the middle of the text: {} vs width {}",
            last.x,
            layout.width
        );
    }

    /// A real glyph rasterizes to an inked mask; a missing glyph does not.
    #[test]
    fn rasterize_inks_a_real_glyph_and_skips_a_missing_one() {
        let font = example_font();
        let layout = layout(&font, "h", 48.0).expect("the font should shape 'h'");
        let id = layout.glyphs[0].id;
        let glyph = rasterize(&font, id, 48.0).expect("'h' should have outlines");
        assert!(glyph.width > 0 && glyph.height > 0);
        assert_eq!(glyph.data.len(), (glyph.width * glyph.height * 4) as usize);
        // The mask must contain actual ink.
        assert!(glyph.data.iter().any(|&a| a > 0), "the mask should be inked");
        // A glyph id that does not exist in the font rasterizes to nothing.
        assert!(rasterize(&font, 9999, 48.0).is_none());
    }

    /// The rasterized mask must be top first: for 'h', the top half of the
    /// mask (the bowl) carries far more ink than the bottom half (the
    /// stem alone). A flipped mask would put the bowl on the bottom.
    #[test]
    fn rasterize_produces_a_top_first_mask() {
        let font = example_font();
        let layout = layout(&font, "h", 48.0).expect("the font should shape 'h'");
        let glyph =
            rasterize(&font, layout.glyphs[0].id, 48.0).expect("'h' should have outlines");
        let w = glyph.width as usize;
        let h = glyph.height as usize;
        let row_ink = |row: usize| -> u32 {
            glyph.data[row * w * 4..(row + 1) * w * 4]
                .chunks(4)
                .map(|px| px[3] as u32)
                .sum()
        };
        let top_half = (0..h / 2).map(row_ink).sum::<u32>();
        let bottom_half = (h / 2..h).map(row_ink).sum::<u32>();
        assert!(
            top_half > bottom_half,
            "'h' should ink its top half (the bowl) more than its bottom half: {} vs {}",
            top_half,
            bottom_half
        );
    }

    /// The shelf packer lays cells left to right, wraps to a new shelf, and
    /// copies the pixels top first.
    #[test]
    fn atlas_packs_glyphs_left_to_right_top_first() {
        fn mask(width: u32, height: u32, alpha: u8) -> RasterGlyph {
            RasterGlyph {
                data: vec![alpha; (width * height * 4) as usize],
                width,
                height,
                left: 0,
                top: 0,
            }
        }
        let mut atlas = Atlas::default();
        let a = mask(10, 4, 255);
        let b = mask(6, 8, 128);
        let c = mask(20, 3, 64);
        atlas.insert_many(&[(1, a), (2, b), (3, c)]);
        assert!(atlas.has(1) && atlas.has(2) && atlas.has(3));
        let c1 = atlas.cell(1).expect("glyph 1 should be packed");
        assert_eq!((c1.x, c1.y), (0, 0));
        let c2 = atlas.cell(2).expect("glyph 2 should be packed");
        assert_eq!((c2.x, c2.y), (10, 0));
        let c3 = atlas.cell(3).expect("glyph 3 should be packed");
        // 10 + 6 + 20 = 36 <= 512, so glyph 3 stays on the first shelf.
        assert_eq!((c3.x, c3.y), (16, 0));
        let pixel = |cell: &Cell, cx: u32, cy: u32| -> usize {
            (((cell.y + cy) * atlas.width + cell.x + cx) as usize) * 4
        };
        // The pixels landed at the cell's top-left corner, top row first.
        assert_eq!(&atlas.data[pixel(c1, 0, 0)..pixel(c1, 0, 0) + 4], &[255; 4]);
        assert_eq!(&atlas.data[pixel(c2, 0, 0)..pixel(c2, 0, 0) + 4], &[128; 4]);
        assert_eq!(&atlas.data[pixel(c3, 5, 1)..pixel(c3, 5, 1) + 4], &[64; 4]);
        // Packed space outside the cells stays transparent black.
        assert_eq!(&atlas.data[pixel(c1, 0, 5)..pixel(c1, 0, 5) + 4], &[0; 4]);
    }

    /// A glyph wider than the atlas is skipped; the cursor still moves on.
    #[test]
    fn atlas_skips_glyphs_that_do_not_fit() {
        fn mask(width: u32, height: u32) -> RasterGlyph {
            RasterGlyph {
                data: vec![255; (width * height * 4) as usize],
                width,
                height,
                left: 0,
                top: 0,
            }
        }
        let mut atlas = Atlas::default();
        atlas.insert_many(&[(1, mask(600, 4)), (2, mask(10, 4))]);
        assert!(!atlas.has(1), "the wide glyph should be skipped");
        assert!(atlas.has(2), "the following glyph should still pack");
        let c2 = atlas.cell(2).expect("glyph 2 should be packed");
        assert_eq!((c2.x, c2.y), (0, 0));
    }
}
