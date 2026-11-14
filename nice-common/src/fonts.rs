//! One icon font for the family, owned by the crate the family already shares.
//!
//! `FluentSystemIcons-Regular.ttf` lived in `nice-tuner/assets/fonts/` while six
//! plugins and the rack embedded it from beside their own icons, by a path no
//! manifest can call a dependency. `tools/gate-plan.py` grew a scan for
//! `include_bytes!` precisely because cargo cannot see such edges: before it, a
//! change to that one file planned as a tuner change left the seven suites that
//! bake the font in standing aside. Fira Code, the text font, has always come
//! from the shared `assets/fonts/` at the root; the icon font was the one asset
//! everybody used and one plugin owned.
//!
//! So the file, its MIT notice and its provenance README moved here, beside the
//! widgets that take a font as an argument, and the path no longer leaves the
//! package that embeds it. `cargo xtask bundle` reads the notice out of this
//! directory and writes it into every bundle the workspace builds.
use std::sync::{Arc, OnceLock};

/// The family's icon font, embedded once: unmodified Fluent System Icons
/// Regular, shared as `Arc<[u8]>` so every editor hands the same bytes to the
/// renderer and the shapes behind them are shared too.
///
/// The settings cogwheel is `ic_fluent_settings_24_regular`, U+F6AA.
/// `assets/fonts/README.md` carries the upstream commit and the hash of these
/// exact bytes; `assets/fonts/LICENSE-FluentSystemIcons.txt` is the MIT notice
/// that has to travel with them.
pub fn icon_font() -> Arc<[u8]> {
    static BYTES: &[u8] = include_bytes!("../assets/fonts/FluentSystemIcons-Regular.ttf");
    static FONT: OnceLock<Arc<[u8]>> = OnceLock::new();
    FONT.get_or_init(|| Arc::from(BYTES)).clone()
}

#[cfg(test)]
mod tests {
    use super::icon_font;
    use std::sync::Arc;

    /// The four bytes of a table tag in an sfnt table directory.
    fn tag(font: &[u8], at: usize) -> [u8; 4] {
        font[at..at + 4].try_into().unwrap()
    }

    /// Walk the table directory of the bytes actually embedded, and return the
    /// tags it lists.
    ///
    /// A `font()` that hands back 2.9 MB of something else compiles, links, and
    /// shows squares. The README's SHA-256 is the authority on *which* font
    /// this is; this is the cheaper question — that the embedded bytes are a
    /// TrueType font with the tables a text run needs, read from the file this
    /// crate embeds rather than the one the prose describes.
    fn tables(font: &[u8]) -> Vec<[u8; 4]> {
        assert!(font.len() > 12, "the embedded icon font is truncated");
        assert_eq!(
            &font[0..4],
            &[0x00, 0x01, 0x00, 0x00],
            "the embedded icon font is not a TrueType font"
        );
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        assert!(count > 0, "the embedded icon font declares no tables");
        (0..count).map(|i| tag(font, 12 + i * 16)).collect()
    }

    #[test]
    fn the_embedded_font_is_a_font() {
        let font = icon_font();
        let listed = tables(&font);
        for want in [b"cmap", b"head", b"hhea", b"hmtx", b"glyf", b"OS/2"] {
            assert!(
                listed.iter().any(|t| t == want),
                "the embedded icon font has no '{}' table; its directory lists {}",
                std::str::from_utf8(want).unwrap(),
                listed.len()
            );
        }
    }

    /// `Arc::from` of the same static every time means one allocation shared by
    /// every panel, which is the reason the family calls this instead of
    /// embedding the file itself: the renderer keys its glyph shapes on the
    /// pointer.
    #[test]
    fn every_call_hands_back_the_same_bytes() {
        let first = icon_font();
        let second = icon_font();
        assert!(Arc::ptr_eq(&first, &second), "each call copied the font");
    }
}
