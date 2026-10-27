//! frost_shaders — reusable shader programs for frost.
//!
//! The engine compiles a game-authored program behind its lighting
//! prelude, binds it exactly like the sprite pipeline, and draws it in
//! place of the sprite on any shape that wears it — the engine's
//! `frost::Shape::with_program` and `Shape::painted_by`. This
//! crate is the shelf for such programs: each one is a plain
//! `&'static str` include_str!'d into the binary, and each signs the
//! engine's contract in a test — `frost::LIGHTING_WGSL` prepended,
//! `vs_main`, `fs_main` and `fs_emissive` provided, the sprite's
//! bindings expected at group 0 — so a program breaks at `cargo test`,
//! not in somebody's dead frame.
//!
//! The shelf so far:
//!
//! - [`DISSOLVE`] eats a sprite tile from a noise field inward, with a
//!   burning edge. Its few floats of effect ride ordinary scene-node
//!   fields, so driving one costs no engine code at all. The smallest
//!   host worth running is the example:
//!
//!   ```text
//!   cargo run -p frost_shaders --example dissolve
//!   ```

/// The dig's dissolve: a sprite tile eaten from the edges inward by a
/// noise band, its bite edge glowing warm, gone by the moment
/// progress reaches 1.0.
///
/// The uniforms are the sprite's — 112 bytes the engine packs from an
/// ordinary `frost::SceneNode` — and what they mean is the program's
/// own saying:
///
/// - `emission.r` — progress: 0.0 whole, 1.0 eaten through;
/// - `emission.g` — a per-tile seed, so no two holes are eaten alike;
/// - `emission.b` — softness of the eat band's edge;
/// - `emission.a` — the grain: cycles of the noise lattice across
///   the node, so the host scales the bite to its own size class
///   (antifrost's dig cells ride 5.0; a window-sized node rides
///   hundreds of pixels per clump). Clamped to [2, 96];
/// - `glow` — the tint laid over the tile, arriving whole when
///   `modulate` stays white.
///
/// Progress rides on the CPU side: the engine hands a program no
/// clock, so the game's `dt` is the clock, and a dissolve is over
/// exactly when the game says it is.
pub const DISSOLVE: &str = include_str!("../shaders/dissolve.wgsl");

#[cfg(test)]
mod tests {
    /// The program must compile behind the engine's lighting prelude
    /// exactly as the engine's own lit kinds do — parse and
    /// device-side validation, the two stages wgpu runs. If the
    /// engine's contract (`LIGHTING_WGSL` prepended, the two fragment
    /// entry points, the sprite bindings) ever moves, this fails on the
    /// shelf rather than inside the next app that borrows the program.
    /// Gated as `frost::validate_wgsl` itself is: naga stands ready in
    /// native debug builds. The import rides that same gate by sitting
    /// inside the test, because a module-level `use super::*` is dead
    /// code in a release build — this is the module's only test and it
    /// is compiled out there — and
    /// `cargo clippy --release --all-targets -- -D warnings` fails a
    /// clean tree on the strength of it.
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    #[test]
    fn the_dissolve_program_keeps_the_contract() {
        use super::DISSOLVE;

        let joined = format!("{}\n{DISSOLVE}", frost::LIGHTING_WGSL);
        if let Err(err) = frost::validate_wgsl(&joined) {
            panic!("dissolve.wgsl fails behind the prelude: {err}");
        }
    }
}
