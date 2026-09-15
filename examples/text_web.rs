//! The web version of the `text` example: the same scene (a swaying
//! circle with "Sub" on top) built for a web browser instead of a
//! desktop window.
//!
//! A browser has no file system, so the font cannot be loaded from
//! `assets/fonts/` at runtime. It is embedded into the binary with
//! `include_bytes!` and built with [`frost::Shape::text_bytes`] instead.
//!
//! Building and running (from the crate root; the machine needs network
//! access for the target and the wasm-bindgen tooling):
//!
//! ```text
//! rustup target add wasm32-unknown-unknown
//! cargo build --example text_web --target wasm32-unknown-unknown --release
//!
//! # The wasm-bindgen CLI must exactly match the wasm-bindgen crate
//! # version recorded in Cargo.lock.
//! cargo install wasm-bindgen-cli --version 0.2.XX
//!
//! wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/examples/text_web.wasm
//!
//! cd web
//! python -m http.server 8080
//! # open http://localhost:8080
//! ```
//!
//! The page needs a browser with WebGPU (Chrome/Edge 113+, Firefox 141+,
//! Safari 26+). Panics are printed to the browser console via
//! `console_error_panic_hook`.
//!
//! The example targets wasm32-unknown-unknown. When cargo builds it for a
//! native target instead (for example `cargo test`, which compiles every
//! example), the wasm-specific parts are cfg'd out and it compiles to an
//! empty binary.

/// The same font the native example loads from disk, embedded instead.
#[cfg(target_arch = "wasm32")]
const FONT: &[u8] = include_bytes!("../assets/fonts/JameGem08_2026-Regular.ttf");

#[cfg(target_arch = "wasm32")]
struct Demo {
    /// Elapsed time in seconds.
    t: f32,
}

#[cfg(target_arch = "wasm32")]
impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The whole circle (and the text riding on it) drifts up and down
        // around the screen center, like any other node in the tree.
        let circle = &mut ctx.scene().root.children[0];
        circle.transform = frost::Transform::translate(0.0, (self.t * 0.5).sin() * 20.0);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // A panic that reaches here shows up in the browser console instead of
    // the module dying silently.
    console_error_panic_hook::set_once();

    let sub = match frost::Shape::text_bytes(FONT, "Sub", 48.0) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            return;
        }
    };

    let scene = frost::Scene::new(frost::SceneNode {
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                shape: Some(sub),
                ..Default::default()
            })],
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { t: 0.0 }) {
        log::error!("frost failed: {err}");
    }
}

/// Native builds of this example are empty: everything above is gated
/// behind `target_arch = "wasm32"`.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
