# `dogs_name`

A four-frame monster flipbook from this folder's `assets/sprites`
(`Monster1.png` … `Monster4.png`, each 1920×1080 RGBA).

| Example | What it shows | Run with |
|---|---|---|
| `dogs_name` | whole-image sprite animation: the four frames are loaded once at startup and swapped into one sprite node every 1/8 s (8 fps, wrapping forever) — an `Arc` clone per frame, not a re-decode. The cycle is a 1-2-1-4 gait: `Monster3.png` is an intentional duplicate of `Monster1.png`, the stand pose being the gait's two "up" beats between the half-squat (frame 2) and the deep crouch (frame 4). The window opens at a true physical 1920x1080 on any display via `Config::window_size_px` (960x540 points on a 2x Retina screen), the fit scale is computed from the live window size each frame, and a gentle float, sway, and breathing keeps the monster from sitting dead still on top of the art's own squash | `cargo run --example dogs_name` |

Because the folder is not named `main.rs`, the example is declared
explicitly in `Cargo.toml` (cargo only auto-discovers `examples/*.rs` and
`examples/*/main.rs`).

## Patterns it shows

- **Flipbook by shape swap.** The frames live in the `Process`, not the
  scene; each frame tick assigns `node.shape = Some(frames[i].clone())`.
  Sprite pixels are shared behind an `Arc`, so the swap is cheap.
- **Fit scale from the art.** `(window / texture).min()` on the real
  texture size keeps full-frame art inside the window whatever its
  resolution, and the idle breathing multiplies that same fit scale.
