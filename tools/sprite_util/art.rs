//! Image plumbing: PNG codec round trips, alpha bounds, the
//! checkerboard texture, and the asset-path helpers.
// The subjects this one reads.
use image::ImageEncoder;

/// The checker cell's edge, in window pixels: the pattern never scales
/// with the sprite — one cell is always 16 pixels on screen.
pub(crate) const CHECK_CELL: f32 = 16.0;

/// The checker's default light grey level, 0.0-1.0. The defaults are
/// deliberately near-black: the pattern must read as a checkerboard
/// without becoming the brightest thing in a dark room — the greys
/// stay live sliders in the View menu for whoever wants them louder.
pub(crate) const GREY_LIGHT: f32 = 0.10;

/// The checker's default dark grey level, 0.0-1.0.
pub(crate) const GREY_DARK: f32 = 0.05;

/// The bounding box of the pixels with a non-zero alpha, as `(x, y, w, h)`;
/// `None` when the image is fully transparent.
pub(crate) fn alpha_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] != 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }
}

/// Encode an RGBA image to PNG bytes in memory.
pub(crate) fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(png)
}

/// The file's name for the dialogs and the logs.
pub(crate) fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string()
}

/// The native "already exists — overwrite?" question, parented to the
/// window like every other dialog.
pub(crate) fn confirm_overwrite(ctx: &frost::Context, file: &str) -> bool {
    let mut dialog = rfd::MessageDialog::new()
        .set_title("sprite_util")
        .set_level(rfd::MessageLevel::Warning)
        .set_description(format!("'{file}' already exists. Overwrite it?"))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(window) = ctx.window() {
        dialog = dialog.set_parent(window);
    }
    dialog.show() == rfd::MessageDialogResult::Yes
}

/// Builds the checker backdrop's sprite: one texture pixel per cell —
/// `cw` by `ch` pixels, the light and dark greys alternating — encoded to
/// PNG in memory. The nearest-neighbor sampler then keeps the cell edges
/// hard when the sprite is scaled to cover the region.
pub(crate) fn checker_shape(cw: u32, ch: u32, light: f32, dark: f32) -> Option<frost::Shape> {
    let l = (light.clamp(0.0, 1.0) * 255.0).round() as u8;
    let d = (dark.clamp(0.0, 1.0) * 255.0).round() as u8;
    // Row 0 is the top row, so the upper-left cell is light.
    let mut pixels = Vec::with_capacity((cw * ch * 4) as usize);
    for y in 0..ch {
        for x in 0..cw {
            let grey = if (x + y) % 2 == 0 { l } else { d };
            pixels.extend_from_slice(&[grey, grey, grey, 255]);
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, cw, ch, image::ExtendedColorType::Rgba8)
        .ok()?;
    frost::Shape::sprite_bytes_nearest(&png).ok()
}

/// The repository's HUD font, relative to the repository root — the same
/// monospaced variable font the diagnostics overlay uses.
pub(crate) const HUD_FONT: &str = "assets/fonts/FiraCode-VariableFont_wght.ttf";

/// The repository root, resolved from this package's manifest directory.
/// The tool is a workspace member at `tools/sprite_util`, so
/// `CARGO_MANIFEST_DIR` is *not* the repository root — it is two levels
/// below it — and every asset path must be built from the root, never
/// from the manifest directory directly. `the_hud_font_is_where_the_asset_helper_points`
/// checks this at test time: the wrong number of `..` is otherwise a
/// silent re-point that only fails when the tool starts up.
pub(crate) fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the tool lives at tools/sprite_util, under a `tools` parent")
        .parent()
        .expect("and `tools` itself has the repository root as its parent")
        .to_path_buf()
}

/// One asset of the repository — `assets/...`, relative to the repository
/// root — as an absolute path. Every runtime asset load goes through
/// here, so the root is pinned in exactly one place.
pub(crate) fn asset_path(rel: &str) -> std::path::PathBuf {
    repo_root().join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hud_font_is_where_the_asset_helper_points() {
        // `CARGO_MANIFEST_DIR` names `tools/sprite_util`, not the
        // repository root, so the font path is a runtime one built by
        // `asset_path` — and a wrong root is invisible to the compiler.
        // This is the loud version of the startup failure the tool would
        // otherwise meet with a window that never opens.
        let font = asset_path(HUD_FONT);
        assert!(
            font.is_file(),
            "the HUD font is not where `asset_path` points: {}",
            font.display()
        );
    }

    #[test]
    fn alpha_bbox_bounds_the_visible_pixels() {
        let mut buf = image::RgbaImage::from_pixel(20, 10, image::Rgba([0, 0, 0, 0]));
        for (x, y, p) in buf.enumerate_pixels_mut() {
            *p = if (3..7).contains(&x) && (2..5).contains(&y) {
                image::Rgba([1, 2, 3, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            };
        }
        assert_eq!(alpha_bbox(&buf), Some((3, 2, 4, 3)));
        assert_eq!(alpha_bbox(&image::RgbaImage::new(8, 8)), None);
    }

    #[test]
    fn the_png_round_trip_survives_the_codec() {
        let mut buf = image::RgbaImage::new(4, 3);
        buf.put_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        let png = png_bytes(&buf).expect("encode");
        let back = image::load_from_memory(&png).expect("decode").to_rgba8();
        assert_eq!(back.width(), 4);
        assert_eq!(back.height(), 3);
        assert_eq!(back.get_pixel(2, 1), &image::Rgba([255, 0, 0, 255]));
        assert_eq!(back.get_pixel(0, 0), &image::Rgba([0, 0, 0, 0]));
    }
}
